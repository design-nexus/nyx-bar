//! Nyx Bar window: the focused window's app (its icon and name, from its desktop file)
//! and title, dimmed; on a narrow monitor, the app alone. Hidden on an empty workspace.

use nyx_plugin_sdk::protocol::MonitorInfo;
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin};
use serde_json::Value;

struct Look {
    title: bool,
    max_chars: usize,
}

impl Look {
    fn from(st: &Value) -> Look {
        Look {
            title: st.get("title").and_then(|v| v.as_bool()).unwrap_or(true),
            max_chars: st.get("max_chars").and_then(|v| v.as_i64()).unwrap_or(48).clamp(8, 200) as usize,
        }
    }
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
    t.push('…');
    t
}

/// The title without the app's name on the end ("notes.md — Zed" → "notes.md").
fn bare_title<'a>(title: &'a str, app: &str) -> &'a str {
    for sep in [" — ", " - ", " – "] {
        if let Some(rest) = title.strip_suffix(app).and_then(|t| t.strip_suffix(sep)) {
            return rest;
        }
    }
    title
}

/// Below this width (logical pixels) a monitor's bar shows the app without its title.
const NARROW: i32 = 1440;

fn window(class: Option<&str>, title: Option<&str>, look: &Look, narrow: bool) -> Item {
    let Some(class) = class.filter(|c| !c.is_empty()) else {
        return item("window").hidden(true);
    };
    let app = nyx_bar_apps::lookup(class);
    let title = title.unwrap_or("");
    let mut c = vec![icon_sized(&app.icon, 16), label(app.name.clone()).class("strong")];
    let t = bare_title(title, &app.name);
    if look.title && !narrow && !t.is_empty() && t != app.name {
        c.push(label(short(t, look.max_chars)).class("dim").class("shrink"));
    }
    item("window").view(hbox(c).spacing(8)).tooltip(if title.is_empty() { app.name.clone() } else { title.to_string() })
}

/// The item on each monitor: the title only where there's room.
fn show(plugin: &Plugin<()>, monitors: &[MonitorInfo], last: &(Option<String>, Option<String>), look: &Look) {
    for m in monitors {
        plugin.item(window(last.0.as_deref(), last.1.as_deref(), look, m.width < NARROW).monitor(&m.name));
    }
}

fn main() {
    let (plugin, inputs) = Plugin::<()>::start();
    let mut look = Look::from(plugin.settings());
    let mut monitors = plugin.hello.monitors.clone();
    let mut last: (Option<String>, Option<String>) = (None, None);
    show(&plugin, &monitors, &last, &look);
    for input in inputs {
        match input {
            Input::Desktop(d) => {
                let now = (d.active_class, d.active_title);
                if now != last {
                    last = now;
                    show(&plugin, &monitors, &last, &look);
                }
            }
            Input::Monitors(m) => {
                monitors = m;
                show(&plugin, &monitors, &last, &look);
            }
            Input::Settings(st) => {
                look = Look::from(&st);
                show(&plugin, &monitors, &last, &look);
            }
            Input::Shutdown => break,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_lose_the_app_name_and_shorten() {
        assert_eq!(bare_title("notes.md — Zed", "Zed"), "notes.md");
        assert_eq!(bare_title("Inbox - Mozilla Firefox", "Mozilla Firefox"), "Inbox");
        assert_eq!(bare_title("~/Projects", "Ghostty"), "~/Projects");
        assert_eq!(short("abcdefghij", 5), "abcd…");
        assert_eq!(short("abc", 5), "abc");
    }
}
