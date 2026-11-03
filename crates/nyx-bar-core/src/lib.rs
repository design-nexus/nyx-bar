//! Nyx Bar's logic without GTK.
//!
//! - [`Config`]: `~/.config/nyx/bar.toml`, the sections, height, plugin settings and
//!   limits, and per-monitor layouts.
//! - [`arrange`]: which items go where, from the layout and the items plugins show.
//! - [`should_hide`]: hide on a monitor showing a true-fullscreen window.

use nyx_desktop::{Desktop, Monitor};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const DEFAULT_HEIGHT: i32 = 32;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Sections {
    #[serde(default)]
    pub left: Option<Vec<String>>,
    #[serde(default)]
    pub center: Option<Vec<String>>,
    #[serde(default)]
    pub right: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Config {
    #[serde(flatten)]
    pub sections: Sections,
    #[serde(default = "default_height")]
    pub height: i32,
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

fn default_height() -> i32 {
    DEFAULT_HEIGHT
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

    /// The file's layout, or the default when there's no file; an error when it's broken.
    pub fn load(path: &Path) -> Result<Config, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str::<Config>(&text).map(Config::checked).map_err(|e| format!("{}: {e}", path.display())),
            Err(_) => Ok(Config::default()),
        }
    }

    fn checked(mut self) -> Config {
        self.height = self.height.clamp(20, 64);
        self
    }

    /// The layout for one monitor: a section the monitor sets replaces the main one.
    pub fn sections_for(&self, monitor: &Monitor) -> [Vec<String>; 3] {
        let m = self.monitors.get(&monitor.stable_id()).or_else(|| self.monitors.get(&monitor.name));
        let pick = |own: Option<&Option<Vec<String>>>, main: &Option<Vec<String>>| {
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

/// Place items: each section's entries in order (`plugin` takes all of its items,
/// `plugin/x` the items with that id or of that part); items no entry takes go to the
/// end of their plugin's section.
pub fn arrange(sections: &[Vec<String>; 3], shown: &[Shown]) -> [Vec<(String, String)>; 3] {
    let mut out: [Vec<(String, String)>; 3] = Default::default();
    let mut placed = vec![false; shown.len()];
    for (si, entries) in sections.iter().enumerate() {
        for e in entries {
            let (plugin, sub) = match e.split_once('/') {
                Some((p, s)) => (p, Some(s)),
                None => (e.as_str(), None),
            };
            for (i, s) in shown.iter().enumerate() {
                if placed[i] || s.plugin != plugin {
                    continue;
                }
                if sub.is_none_or(|x| x == s.item || x == s.part) {
                    placed[i] = true;
                    out[si].push((s.plugin.clone(), s.item.clone()));
                }
            }
        }
    }
    for (i, s) in shown.iter().enumerate() {
        if !placed[i] {
            let si = SECTIONS.iter().position(|x| *x == s.section).unwrap_or(2);
            out[si].push((s.plugin.clone(), s.item.clone()));
        }
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

    #[test]
    fn the_default_layout() {
        let c = Config::default();
        assert_eq!(c.height, 32);
        let m = mon("DP-1", 1);
        let [l, c2, r] = c.sections_for(&m);
        assert_eq!(l, vec!["nyx.workspaces"]);
        assert_eq!(c2, vec!["nyx.clock"]);
        assert!(r.contains(&"nyx.system/volume".to_string()));
        assert!(c.enabled_on(&m));
        assert!(!c.enabled_on(&mon("HEADLESS-2", 1)));
    }

    #[test]
    fn monitors_override_sections() {
        let c: Config = toml::from_str("right = [\"a\"]\n[monitors.\"HDMI-A-1\"]\nright = [\"b\"]\nenabled = true\n[monitors.\"HEADLESS-1\"]\n").unwrap();
        let hdmi = mon("HDMI-A-1", 1);
        assert_eq!(c.sections_for(&hdmi)[2], vec!["b"]);
        let other = mon("DP-1", 1);
        assert_eq!(c.sections_for(&other)[2], vec!["a"]);
        assert!(c.enabled_on(&mon("HEADLESS-1", 1)));
    }

    #[test]
    fn arranging() {
        let sections = [vec!["ws".to_string()], vec![], vec!["sys/volume".to_string(), "clock".to_string(), "sys".to_string()]];
        let items = vec![
            shown("sys", "network", "network", "right"),
            shown("sys", "volume", "volume", "right"),
            shown("clock", "time", "main", "center"),
            shown("ws", "workspaces", "main", "left"),
            shown("acme.ping", "ping", "main", "left"),
            shown("tray", "tray-1", "main", "right"),
        ];
        let [l, c, r] = arrange(&sections, &items);
        let names = |v: &[(String, String)]| v.iter().map(|(p, i)| format!("{p}/{i}")).collect::<Vec<_>>();
        assert_eq!(names(&l), vec!["ws/workspaces", "acme.ping/ping"]);
        assert!(c.is_empty());
        assert_eq!(names(&r), vec!["sys/volume", "clock/time", "sys/network", "tray/tray-1"]);
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
}
