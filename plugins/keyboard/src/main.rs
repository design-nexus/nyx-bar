//! Nyx Bar keyboard: the layout in use as a short code ("EN", "DE"), when more than one
//! is set up. A click switches to the next; right click lists them to pick from. From
//! Hyprland's main keyboard (`hyprctl devices -j`), read again on its `activelayout`
//! and `configreloaded` events.

use nyx_plugin_sdk::protocol::EventKind;
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin, spawn};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::time::Duration;

const ITEM: &str = "keyboard";

#[derive(Debug, Clone, PartialEq, Default)]
struct Layouts {
    /// Each layout's code ("us", "de") and variant.
    list: Vec<(String, String)>,
    active: usize,
    /// The active keymap's name ("English (US)").
    name: String,
}

fn parse(devices: &Value) -> Option<Layouts> {
    let kbs = devices.get("keyboards")?.as_array()?;
    let kb = kbs.iter().find(|k| k["main"] == true).or_else(|| kbs.first())?;
    let codes: Vec<&str> = kb["layout"].as_str().unwrap_or("").split(',').map(str::trim).collect();
    let variants: Vec<&str> = kb["variant"].as_str().unwrap_or("").split(',').map(str::trim).collect();
    let list = codes.iter().enumerate().filter(|(_, c)| !c.is_empty()).map(|(i, c)| (c.to_string(), variants.get(i).unwrap_or(&"").to_string())).collect();
    Some(Layouts { list, active: kb["active_layout_index"].as_u64().unwrap_or(0) as usize, name: kb["active_keymap"].as_str().unwrap_or("").to_string() })
}

fn read() -> Option<Layouts> {
    let out = std::process::Command::new("hyprctl").args(["devices", "-j"]).stdin(std::process::Stdio::null()).output().ok()?;
    parse(&serde_json::from_slice(&out.stdout).ok()?)
}

/// XKB's name for a layout or variant ("English (US)", "German (no dead keys)"), from its
/// rules list where the system has one.
fn describe(layout: &str, variant: &str) -> Option<String> {
    let root = std::env::var("XKB_CONFIG_ROOT").unwrap_or_else(|_| "/usr/share/X11/xkb".into());
    let list = std::fs::read_to_string(format!("{root}/rules/evdev.lst")).ok()?;
    describe_in(&list, layout, variant)
}

fn describe_in(list: &str, layout: &str, variant: &str) -> Option<String> {
    let mut section = "";
    for line in list.lines() {
        if let Some(s) = line.strip_prefix("! ") {
            section = s.trim();
            continue;
        }
        let mut w = line.split_whitespace();
        let (Some(key), rest) = (w.next(), w.collect::<Vec<_>>().join(" ")) else { continue };
        match section {
            "layout" if variant.is_empty() && key == layout => return Some(rest),
            "variant" if key == variant => {
                if let Some(name) = rest.strip_prefix(&format!("{layout}: ")) {
                    return Some(name.to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn code(layout: &str) -> String {
    match layout {
        "us" | "gb" => "EN".into(),
        "jp" => "あ".into(),
        l => l.chars().take(3).collect::<String>().to_uppercase(),
    }
}

fn bar_item(l: &Layouts, always: bool) -> Item {
    if l.list.len() < 2 && !always {
        return item(ITEM).hidden(true);
    }
    let c = l.list.get(l.active).map(|(c, _)| code(c)).unwrap_or_else(|| "?".into());
    item(ITEM)
        .view(label(c).class("kbd"))
        .tooltip(format!("{}\nClick: next layout · Right-click: choose", l.name))
        .on_click(&["hyprctl", "switchxkblayout", "all", "next"])
}

fn menu(p: &Plugin<Option<Layouts>>, l: &Layouts) {
    let mut entries: Vec<MenuEntry> = l
        .list
        .iter()
        .enumerate()
        .map(|(i, (c, v))| {
            let name = describe(c, v).unwrap_or_else(|| if v.is_empty() { c.clone() } else { format!("{c} ({v})") });
            let mut e = menu_entry(&format!("layout:{i}"), &format!("{}  {name}", code(c)));
            e.checked = Some(i == l.active);
            e
        })
        .collect();
    entries.push(menu_entry("settings", "Keyboard settings"));
    p.menu(ITEM, entries);
}

/// Hyprland's event socket, if this session has one.
fn events() -> Option<std::os::unix::net::UnixStream> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let run = std::env::var("XDG_RUNTIME_DIR").ok()?;
    std::os::unix::net::UnixStream::connect(format!("{run}/hypr/{sig}/.socket2.sock")).ok()
}

fn main() {
    let (plugin, inputs) = Plugin::<Option<Layouts>>::start();
    let mut always = plugin.setting_bool("always", false);
    let send = plugin.data();
    std::thread::spawn(move || {
        loop {
            send(read());
            match events() {
                Some(sock) => {
                    for line in BufReader::new(sock).lines() {
                        let Ok(line) = line else { break };
                        if line.starts_with("activelayout>>") || line.starts_with("configreloaded>>") {
                            send(read());
                        }
                    }
                }
                None => std::thread::sleep(Duration::from_secs(5)),
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
    plugin.item(item(ITEM).hidden(true));
    let mut last: Option<Layouts> = None;
    for input in inputs {
        match input {
            Input::Data(Some(l)) => {
                if last.as_ref() != Some(&l) {
                    plugin.item(bar_item(&l, always));
                    menu(&plugin, &l);
                    last = Some(l);
                }
            }
            Input::Settings(st) => {
                always = st.get("always").and_then(|v| v.as_bool()).unwrap_or(false);
                if let Some(l) = &last {
                    plugin.item(bar_item(l, always));
                }
            }
            Input::Event(e) if e.kind == EventKind::Action => match e.control.as_deref() {
                Some(c) if c.starts_with("layout:") => {
                    if let Ok(i) = c[7..].parse::<usize>() {
                        spawn(&["hyprctl", "switchxkblayout", "all", &i.to_string()]);
                    }
                }
                Some("settings") => spawn(&["nyx-settings", "--section", "keyboard"]),
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
    use serde_json::json;

    #[test]
    fn reads_the_main_keyboard() {
        let d = json!({ "keyboards": [
            { "main": false, "layout": "us", "variant": "", "active_layout_index": 0, "active_keymap": "English (US)" },
            { "main": true, "layout": "us,de", "variant": ",nodeadkeys", "active_layout_index": 1, "active_keymap": "German" }
        ]});
        let l = parse(&d).unwrap();
        assert_eq!(l.list, vec![("us".into(), "".into()), ("de".into(), "nodeadkeys".into())]);
        assert_eq!(l.active, 1);
        assert_eq!(code("de"), "DE");
        assert_eq!(code("us"), "EN");
        let list = "! layout\n  us              English (US)\n  de              German\n\n! variant\n  nodeadkeys      de: German (no dead keys)\n";
        assert_eq!(describe_in(list, "us", "").as_deref(), Some("English (US)"));
        assert_eq!(describe_in(list, "de", "nodeadkeys").as_deref(), Some("German (no dead keys)"));
        assert_eq!(describe_in(list, "fr", ""), None);
        assert!(!bar_item(&Layouts { list: vec![("us".into(), "".into())], ..Default::default() }, false).visible);
    }
}
