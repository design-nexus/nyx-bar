//! Nyx Bar workspaces: one item per monitor, a button per workspace. The active one is
//! marked, busy ones are bright, urgent ones are red. Click goes there; scrolling over
//! the item moves to the next or previous workspace on that monitor. A virtual monitor
//! (one region of a split screen) shows its own workspaces, numbered 1 to 10.

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

/// A virtual monitor's own workspaces are `base + n`, named `n` (Nyx's Hyprland with
/// `hl.virtual_monitor`); 0 for every other monitor, whose workspaces are 1 to 10.
fn base_of(d: &Desktop, monitor: &str) -> i64 {
    d.workspaces
        .iter()
        .find(|w| w.monitor == monitor && w.id > 100 && USER_WORKSPACES.contains(&(w.id % 100)) && w.name == (w.id % 100).to_string())
        .map_or(0, |w| w.id - w.id % 100)
}

/// Workspace numbers (1 to 10) with windows. A virtual monitor counts its own; the
/// others share 1 to 10, so a busy workspace shows on every monitor's item.
fn busy_numbers(d: &Desktop, monitor: &str, base: i64) -> Vec<i64> {
    let user = |id: i64| USER_WORKSPACES.contains(&(id - base));
    d.workspaces
        .iter()
        .filter(|w| w.windows > 0 && user(w.id) && (base == 0 || w.monitor == monitor))
        .map(|w| w.id - base)
        .collect()
}

fn draw(plugin: &Plugin<Arc<Desktop>>, d: &Desktop, count: i64) {
    for m in d.monitors.iter().filter(|m| !m.disabled) {
        let base = base_of(d, &m.name);
        let busy = busy_numbers(d, &m.name, base);
        let active = m.active_workspace - base;
        let last = busy.iter().copied().chain([count, active.min(*USER_WORKSPACES.end())]).max().unwrap_or(count);
        let mut buttons = vec![];
        for n in 1..=last.min(*USER_WORKSPACES.end()) {
            let id = base + n;
            let urgent = d.windows.iter().any(|w| w.workspace == id && w.urgent);
            let mut b = button(&n.to_string(), Some(&id.to_string()), None).id(format!("ws-{id}")).class("workspace");
            b = if n == active {
                b.class("active")
            } else if urgent {
                b.class("urgent")
            } else if busy.contains(&n) {
                b.class("busy")
            } else {
                b.class("dim")
            };
            buttons.push(b);
        }
        plugin.item(item(ITEM).monitor(&m.name).view(hbox(buttons).spacing(0)).tooltip("Workspaces · scroll to move along"));
    }
}

fn main() {
    let (plugin, inputs) = Plugin::<Arc<Desktop>>::start();
    let mut count = plugin.setting_i64("count", 5).clamp(1, 10);
    let mut wrap = plugin.setting_bool("wrap", false);
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
                draw(&plugin, &d, count);
                desktop = Some(d);
            }
            Input::Settings(s) => {
                count = s.get("count").and_then(|v| v.as_i64()).unwrap_or(5).clamp(1, 10);
                wrap = s.get("wrap").and_then(|v| v.as_bool()).unwrap_or(false);
                if let Some(d) = &desktop {
                    draw(&plugin, d, count);
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
    fn regions_show_their_own_workspaces() {
        let d = Desktop {
            workspaces: vec![ws(1, "1", "eDP-1", 1), ws(3, "3", "DP-3.v1", 2), ws(201, "1", "DP-3.v2", 0), ws(204, "4", "DP-3.v2", 1), ws(250, "250", "eDP-1", 1)],
            ..Default::default()
        };
        assert_eq!(base_of(&d, "DP-3.v2"), 200);
        assert_eq!(base_of(&d, "DP-3.v1"), 0);
        assert_eq!(base_of(&d, "eDP-1"), 0, "a workspace that merely has a high id doesn't make a region");
        assert_eq!(busy_numbers(&d, "DP-3.v2", 200), vec![4]);
        assert_eq!(busy_numbers(&d, "eDP-1", 0), vec![1, 3], "ordinary monitors share 1 to 10 as before");
    }
}
