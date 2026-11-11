//! Nyx Bar workspaces: one item per monitor, a button per workspace, in one of five
//! looks (`look`): `pills` (the active workspace an accent pill holding its windows'
//! icons, others a number over a dot per window), `numbers`, `icons` (busy workspaces as
//! their apps' icons), `dots` and `underline`. Urgent ones are red. Hovering one lists its
//! windows; click goes there; scrolling over the item moves to the next or previous
//! workspace on that monitor. A monitor with workspaces of its own (Omarchy's second
//! screen, a virtual monitor) shows those, 1 to 10.

use nyx_desktop::hypr::Instance;
use nyx_desktop::watch::{Update, watch};
use nyx_desktop::{Desktop, USER_WORKSPACES};
use nyx_plugin_sdk::protocol::{Direction, EventKind};
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin};
use std::sync::Arc;

const ITEM: &str = "workspaces";

fn dispatch(lua: &str, classic: &str) {
    if let Ok(h) = Instance::find()
        && h.dispatch(lua).is_err()
    {
        let _ = h.dispatch(classic);
    }
}

fn go(target: &str) {
    dispatch(&format!("hl.dsp.focus({{ workspace = \"{target}\" }})"), &format!("workspace {target}"));
}

/// Workspace `n` of a monitor is `base + n`: 0 where all monitors share 1 to 10, a
/// block of its own under Omarchy (a second screen's are 11 to 20) or on a virtual
/// monitor (a region's 201 to 210).
fn base_of(d: &Desktop, monitor: &str) -> i64 {
    d.workspace_base(monitor)
}

/// Workspace numbers (1 to 10) with windows. A monitor with workspaces of its own counts
/// those; where all monitors share 1 to 10, a busy one shows on every monitor's item.
fn busy_numbers(d: &Desktop, monitor: &str, base: i64) -> Vec<i64> {
    let own = d.own_workspaces(monitor);
    d.workspaces
        .iter()
        .filter(|w| w.windows > 0 && USER_WORKSPACES.contains(&(w.id - base)) && (!own || w.monitor == monitor))
        .map(|w| w.id - base)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Look {
    Pills,
    Numbers,
    Icons,
    Dots,
    Underline,
}

impl Look {
    fn parse(s: &str) -> Look {
        match s {
            "numbers" => Look::Numbers,
            "icons" => Look::Icons,
            "dots" => Look::Dots,
            "underline" => Look::Underline,
            _ => Look::Pills,
        }
    }
}

/// A workspace's windows, most recently focused first: (class, title).
fn windows_on(d: &Desktop, id: i64) -> Vec<(String, String)> {
    let mut w: Vec<_> = d.windows.iter().filter(|w| w.workspace == id && !w.hidden).collect();
    w.sort_by_key(|w| w.focus_order);
    w.into_iter().map(|w| (w.class.clone(), w.title.clone())).collect()
}

fn tooltip(n: i64, wins: &[(String, String)]) -> String {
    let mut t = format!("Workspace {n}");
    if wins.is_empty() {
        t.push_str(" · empty");
    }
    for (class, title) in wins.iter().take(8) {
        let app = nyx_bar_apps::lookup(class).name;
        if title.is_empty() || *title == app {
            t.push_str(&format!("\n{app}"));
        } else {
            t.push_str(&format!("\n{app} — {title}"));
        }
    }
    t
}

/// Each window's app once, in focus order.
fn apps(wins: &[(String, String)]) -> Vec<nyx_bar_apps::App> {
    let mut out: Vec<nyx_bar_apps::App> = vec![];
    for (class, _) in wins {
        let a = nyx_bar_apps::lookup(class);
        if !out.iter().any(|o| o.icon == a.icon) {
            out.push(a);
        }
    }
    out
}

fn one(look: Look, n: i64, id: i64, state: &str, wins: &[(String, String)]) -> Node {
    let num = || label(n.to_string()).class("ws-num");
    let node = match look {
        Look::Numbers => label(n.to_string()).class("ws-n"),
        Look::Underline => label(n.to_string()).class("ws-u"),
        Look::Dots => label("").class("ws-dot"),
        Look::Pills if state == "active" => {
            let mut c = vec![num()];
            c.extend(apps(wins).iter().take(4).map(|a| icon_sized(&a.icons_symbolic_first(), 13)));
            hbox(c).spacing(6).class("ws-pill")
        }
        Look::Pills => {
            let dots = "•".repeat(wins.len().min(3));
            vbox([num(), label(dots).class("pips")]).spacing(0).class("ws-pill")
        }
        Look::Icons => {
            let mut c = vec![num()];
            c.extend(apps(wins).iter().take(4).map(|a| icon_sized(&a.icons(), 16)));
            hbox(c).spacing(4).class("ws-apps")
        }
    };
    node.class("ws").class(state).id(&format!("ws-{id}")).action(&id.to_string()).tooltip(&tooltip(n, wins))
}

/// For screenshots on a virtual monitor whose workspace is outside 1 to 10: the
/// workspace to draw as the active one (`NYX_BAR_TEST_WORKSPACE`).
fn test_active() -> Option<i64> {
    std::env::var("NYX_BAR_TEST_WORKSPACE").ok()?.parse().ok()
}

fn draw(plugin: &Plugin<Arc<Desktop>>, d: &Desktop, count: i64, look: Look) {
    for m in d.monitors.iter().filter(|m| !m.disabled) {
        let base = base_of(d, &m.name);
        let busy = busy_numbers(d, &m.name, base);
        let active = test_active().unwrap_or(m.active_workspace - base);
        let last = busy.iter().copied().chain([count, active.min(*USER_WORKSPACES.end())]).max().unwrap_or(count);
        let mut nodes = vec![];
        for n in 1..=last.min(*USER_WORKSPACES.end()) {
            let id = base + n;
            let urgent = d.windows.iter().any(|w| w.workspace == id && w.urgent);
            let state = if n == active {
                "active"
            } else if urgent {
                "urgent"
            } else if busy.contains(&n) {
                "busy"
            } else {
                "dim"
            };
            // Busy workspaces only, in the icons look (and the one you're on).
            if look == Look::Icons && state == "dim" {
                continue;
            }
            nodes.push(one(look, n, id, state, &windows_on(d, id)));
        }
        let spacing = match look {
            Look::Dots => 5,
            Look::Numbers | Look::Underline => 0,
            _ => 2,
        };
        plugin.item(item(ITEM).monitor(&m.name).view(hbox(nodes).spacing(spacing).class(&format!("look-{look:?}").to_lowercase())).tooltip("Workspaces · scroll to move along"));
    }
}

fn main() {
    let (plugin, inputs) = Plugin::<Arc<Desktop>>::start();
    let mut count = plugin.setting_i64("count", 5).clamp(1, 10);
    let mut wrap = plugin.setting_bool("wrap", false);
    let mut look = Look::parse(&plugin.setting_str("look", "pills"));
    let send = plugin.data();
    let _watcher = watch(move |u: Update| {
        if u.changes.workspaces || u.changes.active || u.changes.windows || u.changes.monitors {
            send(u.desktop);
        }
    });
    let mut desktop: Option<Arc<Desktop>> = None;
    for input in inputs {
        match input {
            Input::Data(d) => {
                draw(&plugin, &d, count, look);
                desktop = Some(d);
            }
            Input::Settings(s) => {
                count = s.get("count").and_then(|v| v.as_i64()).unwrap_or(5).clamp(1, 10);
                wrap = s.get("wrap").and_then(|v| v.as_bool()).unwrap_or(false);
                look = Look::parse(s.get("look").and_then(|v| v.as_str()).unwrap_or("pills"));
                if let Some(d) = &desktop {
                    draw(&plugin, d, count, look);
                }
            }
            Input::Event(e) => match e.kind {
                EventKind::Action => {
                    if let Some(id) = e.control.as_deref().filter(|c| c.parse::<i64>().is_ok()) {
                        go(id);
                    }
                }
                EventKind::Scroll => {
                    let Some(d) = &desktop else { continue };
                    let Some(m) = e.monitor.as_deref().and_then(|n| d.monitor(n)) else { continue };
                    let up = e.direction == Some(Direction::Up);
                    let base = base_of(d, &m.name);
                    let busy = busy_numbers(d, &m.name, base);
                    let last = busy.iter().copied().chain([count]).max().unwrap_or(count);
                    let cur = m.active_workspace - base;
                    let next = if up { cur - 1 } else { cur + 1 };
                    let next = match (next < 1, next > last) {
                        (true, _) if wrap => last,
                        (_, true) if wrap => 1,
                        (true, _) | (_, true) => continue,
                        _ => next,
                    };
                    go(&(base + next).to_string());
                }
                _ => {}
            },
            Input::Shutdown => break,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nyx_desktop::Workspace;

    fn ws(id: i64, name: &str, monitor: &str, windows: u32) -> Workspace {
        Workspace { id, name: name.into(), monitor: monitor.into(), windows, has_fullscreen: false, tiled_layout: String::new() }
    }

    #[test]
    fn regions_and_omarchy_screens_show_their_own_workspaces() {
        let mut d = Desktop {
            workspaces: vec![ws(1, "1", "eDP-1", 1), ws(3, "3", "DP-3.v1", 2), ws(201, "1", "DP-3.v2", 0), ws(204, "4", "DP-3.v2", 1)],
            ..Default::default()
        };
        d.regions.insert("DP-3.v2".into(), nyx_desktop::VirtualRegion { host: "DP-3".into(), index: 2, count: 2, workspace_base: 200, ..Default::default() });
        assert_eq!(base_of(&d, "DP-3.v2"), 200);
        assert_eq!(base_of(&d, "DP-3.v1"), 0);
        assert_eq!(busy_numbers(&d, "DP-3.v2", 200), vec![4]);
        assert_eq!(busy_numbers(&d, "eDP-1", 0), vec![1, 3], "ordinary monitors share 1 to 10 as before");

        let mut o = Desktop { workspaces: vec![ws(1, "1", "eDP-1", 1), ws(13, "13", "DP-1", 1)], ..Default::default() };
        o.numbering.omarchy_blocks = Some(nyx_desktop::model::Numbering::parse_omarchy("0\teDP-1\tLaptop\n1\tDP-1\tScreen\n"));
        assert_eq!(base_of(&o, "DP-1"), 10);
        assert_eq!(busy_numbers(&o, "DP-1", 10), vec![3]);
        assert_eq!(busy_numbers(&o, "eDP-1", 0), vec![1], "each Omarchy screen counts its own");
    }
}
