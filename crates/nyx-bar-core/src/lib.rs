//! Nyx Bar's logic without GTK.
//!
//! - [`Config`]: the sections, size, plugin settings and limits, and per-monitor
//!   layouts: the built-in layout, then NyxOS's defaults (`/etc/nyxos/defaults/bar.toml`)
//!   and an administrator's, then the user's `~/.config/nyx/bar.toml`.
//! - [`arrange`]: which items go where, from the layout and the items plugins show,
//!   kept in the layout's pill groups.
//! - [`should_hide`]: hide on a monitor showing a true-fullscreen window.

use nyx_desktop::{Desktop, Monitor};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const DEFAULT_HEIGHT: i32 = 32;

/// The sizes the bar comes in: `size = "cozy"` in `bar.toml`.
pub const SIZES: [(&str, i32); 4] = [("compact", 28), ("default", 32), ("cozy", 36), ("spacious", 40)];

/// One entry of a section: a plugin or one of its items, a pill group of them, or `"|"`
/// for a divider.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Entry {
    One(String),
    Group(Vec<String>),
}

impl Entry {
    pub fn is_divider(&self) -> bool {
        matches!(self, Entry::One(e) if e == "|")
    }
}

impl From<&str> for Entry {
    fn from(e: &str) -> Entry {
        Entry::One(e.to_string())
    }
}

/// The bar's NyxOS domain. The bar never writes its file.
pub const DOMAIN: nyx_config::Domain = nyx_config::Domain { name: "bar", version: 0, migrations: &[] };

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Sections {
    #[serde(default)]
    pub left: Option<Vec<Entry>>,
    #[serde(default)]
    pub center: Option<Vec<Entry>>,
    #[serde(default)]
    pub right: Option<Vec<Entry>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Config {
    #[serde(flatten)]
    pub sections: Sections,
    /// One of [`SIZES`]; an unknown name is the default.
    #[serde(default)]
    pub size: Option<String>,
    /// An exact height, which wins over `size`.
    #[serde(default)]
    pub height: Option<i32>,
    /// `gpu` draws with GTK's GPU renderer instead of cairo (more memory).
    #[serde(default)]
    pub renderer: Option<String>,
    /// Hide the bar on a monitor showing a fullscreen window.
    #[serde(default = "yes")]
    pub hide_for_fullscreen: bool,
    #[serde(default)]
    pub monitors: BTreeMap<String, MonitorConfig>,
    /// Settings per plugin; `limits` inside is the plugin's limits.
    #[serde(default)]
    pub plugins: BTreeMap<String, toml::Table>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct MonitorConfig {
    #[serde(flatten)]
    pub sections: Sections,
    /// `false` leaves this monitor without a bar.
    #[serde(default)]
    pub enabled: Option<bool>,
}

fn yes() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        toml::from_str(nyx_plugin::layout::DEFAULT_BAR).expect("the default layout parses")
    }
}

impl Config {
    pub fn path() -> std::path::PathBuf {
        nyx_plugin::paths::layout_file(nyx_plugin::Kind::Bar)
    }

    /// The layout through every layer; an error when one of them is broken (the bar then
    /// keeps the layout it has).
    pub fn load_layered(layers: &nyx_config::Layers) -> Result<Config, String> {
        let mut table: toml::Table = toml::from_str(nyx_plugin::layout::DEFAULT_BAR).expect("the default layout parses");
        nyx_config::merge(&mut table, layers.merged(DOMAIN.name).map_err(|e| format!("{e:#}"))?);
        table.remove(nyx_config::SCHEMA_KEY);
        Config::deserialize(table).map(Config::checked).map_err(|e| format!("{}: {e}", Config::path().display()))
    }

    #[cfg(test)]
    fn load_from_str(text: &str) -> Config {
        toml::from_str::<Config>(text).map(Config::checked).expect("parses")
    }

    /// The file's layout, or the default when there's no file; an error when it's broken.
    pub fn load(path: &Path) -> Result<Config, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str::<Config>(&text).map(Config::checked).map_err(|e| format!("{}: {e}", path.display())),
            Err(_) => Ok(Config::default()),
        }
    }

    fn checked(mut self) -> Config {
        self.height = self.height.map(|h| h.clamp(20, 64));
        self
    }

    /// The size's name: one of [`SIZES`], or `custom` for an exact `height`.
    pub fn size_name(&self) -> &str {
        if self.height.is_some() {
            return "custom";
        }
        let want = self.size.as_deref().unwrap_or("default");
        SIZES.iter().find(|(n, _)| *n == want).map_or("default", |(n, _)| n)
    }

    /// The bar's height in logical pixels.
    pub fn height(&self) -> i32 {
        self.height.unwrap_or_else(|| SIZES.iter().find(|(n, _)| *n == self.size_name()).map_or(DEFAULT_HEIGHT, |(_, h)| *h))
    }

    /// The layout for one monitor: a section the monitor sets replaces the main one.
    pub fn sections_for(&self, monitor: &Monitor) -> [Vec<Entry>; 3] {
        let m = self.monitors.get(&monitor.stable_id()).or_else(|| self.monitors.get(&monitor.name));
        let pick = |own: Option<&Option<Vec<Entry>>>, main: &Option<Vec<Entry>>| {
            own.and_then(|o| o.clone()).or_else(|| main.clone()).unwrap_or_default()
        };
        [
            pick(m.map(|m| &m.sections.left), &self.sections.left),
            pick(m.map(|m| &m.sections.center), &self.sections.center),
            pick(m.map(|m| &m.sections.right), &self.sections.right),
        ]
    }

    /// Whether this monitor gets a bar. Headless outputs don't unless the file says so.
    pub fn enabled_on(&self, monitor: &Monitor) -> bool {
        let m = self.monitors.get(&monitor.stable_id()).or_else(|| self.monitors.get(&monitor.name));
        match m.and_then(|m| m.enabled) {
            Some(e) => e,
            None => !monitor.is_virtual() || m.is_some(),
        }
    }

    /// A plugin's settings: its manifest's defaults, then the user's.
    pub fn settings(&self, id: &str, defaults: &toml::Table) -> serde_json::Value {
        let mut t = defaults.clone();
        if let Some(user) = self.plugins.get(id) {
            for (k, v) in user {
                if k != "limits" {
                    t.insert(k.clone(), v.clone());
                }
            }
        }
        serde_json::to_value(t).unwrap_or_default()
    }

    pub fn limits(&self, id: &str) -> Option<&toml::Table> {
        self.plugins.get(id).and_then(|p| p.get("limits")).and_then(|l| l.as_table())
    }
}

/// An item a plugin shows, for arranging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub plugin: String,
    pub item: String,
    /// The part it belongs to (for `plugin/part` entries).
    pub part: String,
    /// Where its plugin asks to go: `left`, `center` or `right`.
    pub section: String,
}

pub const SECTIONS: [&str; 3] = ["left", "center", "right"];

/// A placed item: its plugin and item ids.
pub type Placed = (String, String);

/// What a section holds, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    Item(Placed),
    /// Items sharing one pill.
    Group(Vec<Placed>),
    Divider,
}

/// Place items: each section's entries in order (`plugin` takes all of its items,
/// `plugin/x` the items with that id or of that part; a group's entries share a pill);
/// items no entry takes go to the end of their plugin's section. Groups with nothing to
/// show are left out, as are dividers with nothing before or after them.
pub fn arrange(sections: &[Vec<Entry>; 3], shown: &[Shown]) -> [Vec<Slot>; 3] {
    let mut out: [Vec<Slot>; 3] = Default::default();
    let mut placed = vec![false; shown.len()];
    let take = |e: &str, placed: &mut Vec<bool>| -> Vec<Placed> {
        let (plugin, sub) = match e.split_once('/') {
            Some((p, s)) => (p, Some(s)),
            None => (e, None),
        };
        let mut got = vec![];
        for (i, s) in shown.iter().enumerate() {
            if !placed[i] && s.plugin == plugin && sub.is_none_or(|x| x == s.item || x == s.part) {
                placed[i] = true;
                got.push((s.plugin.clone(), s.item.clone()));
            }
        }
        got
    };
    for (si, entries) in sections.iter().enumerate() {
        for e in entries {
            match e {
                _ if e.is_divider() => out[si].push(Slot::Divider),
                Entry::One(e) => out[si].extend(take(e, &mut placed).into_iter().map(Slot::Item)),
                Entry::Group(g) => {
                    let items: Vec<Placed> = g.iter().flat_map(|e| take(e, &mut placed)).collect();
                    if !items.is_empty() {
                        out[si].push(Slot::Group(items));
                    }
                }
            }
        }
    }
    for (i, s) in shown.iter().enumerate() {
        if !placed[i] {
            let si = SECTIONS.iter().position(|x| *x == s.section).unwrap_or(2);
            out[si].push(Slot::Item((s.plugin.clone(), s.item.clone())));
        }
    }
    for slots in out.iter_mut() {
        let mut tidy: Vec<Slot> = vec![];
        for slot in slots.drain(..) {
            if slot == Slot::Divider && tidy.last().is_none_or(|l| *l == Slot::Divider) {
                continue;
            }
            tidy.push(slot);
        }
        if tidy.last() == Some(&Slot::Divider) {
            tidy.pop();
        }
        *slots = tidy;
    }
    out
}

/// Whether the bar should hide on `monitor`: a true-fullscreen window (not just a
/// maximized one) on its active workspace.
pub fn should_hide(desktop: &Desktop, monitor: &Monitor) -> bool {
    let ws = monitor.special_workspace.unwrap_or(monitor.active_workspace);
    desktop.windows.iter().any(|w| w.workspace == ws && !w.hidden && w.fullscreen & 2 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;


    fn mon(name: &str, active: i64) -> Monitor {
        Monitor {
            id: 0,
            name: name.into(),
            description: String::new(),
            make: String::new(),
            model: String::new(),
            serial: String::new(),
            width: 1920,
            height: 1080,
            x: 0,
            y: 0,
            scale: 1.0,
            transform: 0,
            refresh_rate: 60.0,
            focused: true,
            disabled: false,
            active_workspace: active,
            special_workspace: None,
            reserved: [0, 0, 0, 0],
        }
    }

    fn win(workspace: i64, fullscreen: u8) -> nyx_desktop::Window {
        nyx_desktop::Window {
            address: format!("{workspace}{fullscreen}"),
            stable_id: String::new(),
            class: "x".into(),
            initial_class: "x".into(),
            title: String::new(),
            workspace,
            monitor: 0,
            floating: false,
            fullscreen,
            at: [0, 0],
            size: [10, 10],
            pid: 1,
            focus_order: 0,
            pinned: false,
            hidden: false,
            urgent: false,
            grouped: Vec::new(),
        }
    }

    fn shown(plugin: &str, item: &str, part: &str, section: &str) -> Shown {
        Shown { plugin: plugin.into(), item: item.into(), part: part.into(), section: section.into() }
    }

    fn e(list: &[&str]) -> Vec<Entry> {
        list.iter().map(|x| Entry::from(*x)).collect()
    }

    #[test]
    fn the_default_layout() {
        let c = Config::default();
        assert_eq!(c.height(), 32);
        assert_eq!(c.size_name(), "default");
        let m = mon("DP-1", 1);
        let [l, c2, r] = c.sections_for(&m);
        assert_eq!(l[0], Entry::Group(vec!["nyx.launcher".into(), "nyx.workspaces".into()]));
        assert_eq!(c2, vec![Entry::Group(vec!["nyx.clock".into()])]);
        assert!(r.contains(&Entry::Group(vec!["nyx.system".into()])));
        assert!(c.enabled_on(&m));
        assert!(!c.enabled_on(&mon("HEADLESS-2", 1)));
    }

    #[test]
    fn sizes_and_heights() {
        let c: Config = toml::from_str("size = \"cozy\"").unwrap();
        assert_eq!((c.height(), c.size_name()), (36, "cozy"));
        let c: Config = toml::from_str("size = \"huge\"").unwrap();
        assert_eq!((c.height(), c.size_name()), (32, "default"));
        let c = Config::load_from_str("size = \"compact\"\nheight = 99\n");
        assert_eq!((c.height(), c.size_name()), (64, "custom"), "a height wins, clamped");
    }

    #[test]
    fn monitors_override_sections() {
        let c: Config = toml::from_str("right = [\"a\"]\n[monitors.\"HDMI-A-1\"]\nright = [\"b\"]\nenabled = true\n[monitors.\"HEADLESS-1\"]\n").unwrap();
        let hdmi = mon("HDMI-A-1", 1);
        assert_eq!(c.sections_for(&hdmi)[2], e(&["b"]));
        let other = mon("DP-1", 1);
        assert_eq!(c.sections_for(&other)[2], e(&["a"]));
        assert!(c.enabled_on(&mon("HEADLESS-1", 1)));
    }

    fn names(v: &[Slot]) -> Vec<String> {
        v.iter()
            .map(|s| match s {
                Slot::Item((p, i)) => format!("{p}/{i}"),
                Slot::Group(g) => format!("[{}]", g.iter().map(|(p, i)| format!("{p}/{i}")).collect::<Vec<_>>().join(" ")),
                Slot::Divider => "|".into(),
            })
            .collect()
    }

    #[test]
    fn arranging() {
        let sections = [e(&["ws"]), vec![], e(&["sys/volume", "clock", "sys"])];
        let items = vec![
            shown("sys", "network", "network", "right"),
            shown("sys", "volume", "volume", "right"),
            shown("clock", "time", "main", "center"),
            shown("ws", "workspaces", "main", "left"),
            shown("acme.ping", "ping", "main", "left"),
            shown("tray", "tray-1", "main", "right"),
        ];
        let [l, c, r] = arrange(&sections, &items);
        assert_eq!(names(&l), vec!["ws/workspaces", "acme.ping/ping"]);
        assert!(c.is_empty());
        assert_eq!(names(&r), vec!["sys/volume", "clock/time", "sys/network", "tray/tray-1"]);
    }

    #[test]
    fn arranging_groups_and_dividers() {
        let sections: [Vec<Entry>; 3] = [
            toml::from_str::<Sections>("left = [\"|\", [\"launcher\", \"ws\"], \"|\", \"|\", \"win\", \"|\"]").unwrap().left.unwrap(),
            toml::from_str::<Sections>("center = [[\"gone\"], \"|\"]").unwrap().center.unwrap(),
            toml::from_str::<Sections>("right = [[\"sys\"], [\"weather\", \"stats\"]]").unwrap().right.unwrap(),
        ];
        let items = vec![
            shown("ws", "workspaces", "main", "left"),
            shown("win", "title", "main", "left"),
            shown("sys", "network", "network", "right"),
            shown("sys", "battery", "battery", "right"),
            shown("stats", "cpu", "main", "right"),
        ];
        let [l, c, r] = arrange(&sections, &items);
        assert_eq!(names(&l), vec!["[ws/workspaces]", "|", "win/title"], "launcher isn't running; stray dividers go");
        assert!(c.is_empty(), "an empty group goes: {c:?}");
        assert_eq!(names(&r), vec!["[sys/network sys/battery]", "[stats/cpu]"]);
    }

    #[test]
    fn settings_and_limits() {
        let c: Config = toml::from_str("[plugins.\"nyx.clock\"]\nformat = \"%H\"\n[plugins.\"nyx.clock\".limits]\nmemory = \"64M\"\n").unwrap();
        let defaults: toml::Table = toml::from_str("format = \"%H:%M\"\nseconds = false").unwrap();
        let s = c.settings("nyx.clock", &defaults);
        assert_eq!(s["format"], "%H");
        assert_eq!(s["seconds"], false);
        assert!(s.get("limits").is_none());
        assert!(c.limits("nyx.clock").is_some());
    }

    #[test]
    fn hides_for_true_fullscreen_only() {
        let m = mon("DP-1", 3);
        let mut d = Desktop::default();
        d.windows.push(win(3, 1));
        assert!(!should_hide(&d, &m));
        d.windows.push(win(4, 2));
        assert!(!should_hide(&d, &m));
        d.windows.push(win(3, 2));
        assert!(should_hide(&d, &m));
    }

    #[test]
    fn nyxos_defaults_come_between_the_built_in_layout_and_the_users() {
        let d = tempfile::tempdir().unwrap();
        let layers = nyx_config::Layers { defaults: d.path().join("d"), system: d.path().join("s"), user: d.path().join("u") };
        for dir in [&layers.defaults, &layers.user] {
            std::fs::create_dir_all(dir).unwrap();
        }
        assert_eq!(Config::load_layered(&layers).unwrap(), Config::default());
        std::fs::write(layers.defaults.join("bar.toml"), "schema = 1\nsize = \"cozy\"\n").unwrap();
        let c = Config::load_layered(&layers).unwrap();
        assert_eq!(c.height(), 36);
        assert_eq!(c.sections, Config::default().sections, "the built-in sections stay");
        std::fs::write(layers.user.join("bar.toml"), "height = 28\n").unwrap();
        assert_eq!(Config::load_layered(&layers).unwrap().height(), 28);
        std::fs::write(layers.user.join("bar.toml"), "height = = 2\n").unwrap();
        assert!(Config::load_layered(&layers).is_err());
        assert!(layers.user.join("bar.toml").exists(), "a broken hand edit is left where it is");
    }
}
