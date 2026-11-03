//! Nyx Bar workspaces: one item per monitor, a button per workspace. The active one is
//! marked, busy ones are bright, urgent ones are red. Click goes there; scrolling over
//! the item moves to the next or previous workspace on that monitor.

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

fn draw(plugin: &Plugin<Arc<Desktop>>, d: &Desktop, count: i64) {
    for m in d.monitors.iter().filter(|m| !m.disabled) {
        let busy: Vec<i64> = d.workspaces.iter().filter(|w| w.is_user() && w.windows > 0).map(|w| w.id).collect();
        let last = busy.iter().copied().chain([count, m.active_workspace.min(*USER_WORKSPACES.end())]).max().unwrap_or(count);
        let mut buttons = vec![];
        for id in 1..=last.min(*USER_WORKSPACES.end()) {
            let urgent = d.windows.iter().any(|w| w.workspace == id && w.urgent);
            let mut b = button(&id.to_string(), Some(&id.to_string()), None).id(&format!("ws-{id}")).class("workspace");
            b = if id == m.active_workspace {
                b.class("active")
            } else if urgent {
                b.class("urgent")
            } else if busy.contains(&id) {
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
                    let busy: Vec<i64> = d.workspaces.iter().filter(|w| w.is_user() && w.windows > 0).map(|w| w.id).collect();
                    let last = busy.iter().copied().chain([count]).max().unwrap_or(count);
                    let cur = m.active_workspace;
                    let next = if up { cur - 1 } else { cur + 1 };
                    let next = match (next < 1, next > last) {
                        (true, _) if wrap => last,
                        (_, true) if wrap => 1,
                        (true, _) | (_, true) => continue,
                        _ => next,
                    };
                    go(&next.to_string());
                }
                _ => {}
            },
            Input::Shutdown => break,
            _ => {}
        }
    }
}
