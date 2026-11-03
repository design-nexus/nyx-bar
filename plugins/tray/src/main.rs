//! Nyx Bar tray: StatusNotifierItem icons from other apps. When another program
//! already runs the watcher (Quickshell on Omarchy), this registers as a host with it;
//! otherwise it is the watcher. Each icon is an item; its DBusMenu becomes the item's
//! menu.

mod png;

use nyx_plugin_sdk::protocol::{EventKind, MenuEntry};
use nyx_plugin_sdk::services::dbus::{self, Connection, Props, get_all, plain, prop_bool, prop_str, str_of};
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zbus::zvariant::{OwnedValue, Value};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const MENU_IFACE: &str = "com.canonical.dbusmenu";

/// The watcher, when no one else runs one.
struct Watcher {
    items: Arc<Mutex<Vec<String>>>,
    changed: std::sync::mpsc::Sender<()>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(&self, service: String, #[zbus(header)] header: zbus::message::Header<'_>) {
        let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
        // Ayatana items send their object path; others their bus name.
        let full = if service.starts_with('/') { format!("{sender}{service}") } else { service };
        let mut items = self.items.lock().unwrap();
        if !items.contains(&full) {
            items.push(full);
        }
        let _ = self.changed.send(());
    }

    fn register_status_notifier_host(&self, _service: String) {}

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().unwrap().clone()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}

fn split(service: &str) -> (String, String) {
    match service.find('/') {
        Some(i) => (service[..i].to_string(), service[i..].to_string()),
        None => (service.to_string(), "/StatusNotifierItem".to_string()),
    }
}

fn name_has_owner(conn: &Connection, name: &str) -> bool {
    conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(name,))
        .ok()
        .and_then(|r| r.body().deserialize::<bool>().ok())
        .unwrap_or(false)
}

#[derive(Debug, Clone, PartialEq)]
struct Tray {
    service: String,
    bus: String,
    path: String,
    title: String,
    passive: bool,
    icon: Icon,
    menu: Option<String>,
    item_is_menu: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Icon {
    Name(String),
    File(String),
    Png(String),
    None,
}

fn find_icon_file(dir: &str, name: &str) -> Option<String> {
    fn walk(d: &Path, name: &str, depth: u32) -> Option<String> {
        if depth > 4 {
            return None;
        }
        for e in std::fs::read_dir(d).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Some(f) = walk(&p, name, depth + 1) {
                    return Some(f);
                }
            } else if p.file_stem().is_some_and(|s| s == name) && p.extension().is_some_and(|x| x == "png" || x == "svg") {
                return Some(p.to_string_lossy().into_owned());
            }
        }
        None
    }
    walk(Path::new(dir), name, 0)
}

/// The largest pixmap of `a(iiay)`.
fn pixmap(v: &Value) -> Option<String> {
    let Value::Array(arr) = plain(v) else { return None };
    let mut best: Option<(i32, i32, Vec<u8>)> = None;
    for entry in arr.iter() {
        let Value::Structure(s) = plain(entry) else { continue };
        let f = s.fields();
        let (Some(Value::I32(w)), Some(Value::I32(h)), Some(Value::Array(bytes))) = (f.first().map(plain), f.get(1).map(plain), f.get(2).map(plain)) else {
            continue;
        };
        let data: Vec<u8> = bytes.iter().filter_map(|b| if let Value::U8(x) = plain(b) { Some(*x) } else { None }).collect();
        if *w <= 0 || *h <= 0 || data.len() != (*w * *h * 4) as usize || *w > 256 {
            continue;
        }
        if best.as_ref().is_none_or(|(bw, _, _)| *w > *bw) {
            best = Some((*w, *h, data));
        }
    }
    let (w, h, data) = best?;
    Some(png::base64(&png::encode(w as u32, h as u32, &png::argb_to_rgba(&data))))
}

fn read_item(conn: &Connection, service: &str) -> Option<Tray> {
    let (bus, path) = split(service);
    let p: Props = get_all(conn, &bus, &path, ITEM_IFACE)?;
    let theme_path = prop_str(&p, "IconThemePath").filter(|s| !s.is_empty());
    let status = prop_str(&p, "Status").unwrap_or_default();
    let name = if status == "NeedsAttention" { prop_str(&p, "AttentionIconName") } else { None }
        .or_else(|| prop_str(&p, "IconName"))
        .filter(|n| !n.is_empty());
    let icon = match (&name, &theme_path) {
        (Some(n), Some(dir)) => find_icon_file(dir, n).map(Icon::File).unwrap_or(Icon::Name(n.clone())),
        (Some(n), None) if n.starts_with('/') => Icon::File(n.clone()),
        (Some(n), None) => Icon::Name(n.clone()),
        (None, _) => p.get("IconPixmap").and_then(|v| pixmap(v)).map(Icon::Png).unwrap_or(Icon::None),
    };
    let tooltip = p.get("ToolTip").and_then(|v| match plain(v) {
        Value::Structure(s) => s.fields().get(2).and_then(str_of).filter(|t| !t.is_empty()),
        _ => None,
    });
    let title = tooltip.or_else(|| prop_str(&p, "Title")).or_else(|| prop_str(&p, "Id")).unwrap_or_default();
    Some(Tray {
        service: service.to_string(),
        bus: bus.clone(),
        path,
        title,
        passive: status == "Passive",
        icon,
        menu: prop_str(&p, "Menu").filter(|m| m.len() > 1),
        item_is_menu: prop_bool(&p, "ItemIsMenu").unwrap_or(false),
    })
}

fn menu_entry(conn_bus: &str, menu_path: &str, v: &Value) -> Option<MenuEntry> {
    let Value::Structure(s) = plain(v) else { return None };
    let f = s.fields();
    let id = match f.first().map(plain) {
        Some(Value::I32(i)) => *i,
        _ => return None,
    };
    let props = f.get(1).map(|p| dbus::dict_of(p)).unwrap_or_default();
    let get = |k: &str| props.get(k).and_then(|v| str_of(v));
    let flag = |k: &str| props.get(k).and_then(|v| dbus::bool_of(v));
    if flag("visible") == Some(false) {
        return None;
    }
    if get("type").as_deref() == Some("separator") {
        return Some(MenuEntry { separator: true, enabled: true, ..Default::default() });
    }
    let children: Vec<MenuEntry> = match f.get(2).map(plain) {
        Some(Value::Array(a)) => a.iter().filter_map(|c| menu_entry(conn_bus, menu_path, c)).collect(),
        _ => vec![],
    };
    let checked = get("toggle-type").filter(|t| !t.is_empty()).map(|_| props.get("toggle-state").and_then(|v| dbus::num_of(v)) == Some(1.0));
    Some(MenuEntry {
        id: format!("menu:{conn_bus}|{menu_path}|{id}"),
        label: get("label").unwrap_or_default().replace("__", "\u{1}").replace('_', "").replace('\u{1}', "_"),
        icon: get("icon-name").filter(|i| !i.is_empty()),
        checked,
        enabled: flag("enabled").unwrap_or(true),
        separator: false,
        children,
    })
}

fn read_menu(conn: &Connection, bus: &str, path: &str) -> Vec<MenuEntry> {
    let _ = conn.call_method(Some(bus), path, Some(MENU_IFACE), "AboutToShow", &(0i32,));
    let Ok(reply) = conn.call_method(Some(bus), path, Some(MENU_IFACE), "GetLayout", &(0i32, -1i32, Vec::<String>::new())) else {
        return vec![];
    };
    let Ok((_, layout)) = reply.body().deserialize::<(u32, OwnedValue)>() else {
        // Some send the structure, not a variant.
        return match reply.body().deserialize::<(u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>))>() {
            Ok((_, (_, _, children))) => children.iter().filter_map(|c| menu_entry(bus, path, c)).collect(),
            Err(_) => vec![],
        };
    };
    match plain(&layout) {
        Value::Structure(s) => match s.fields().get(2).map(plain) {
            Some(Value::Array(a)) => a.iter().filter_map(|c| menu_entry(bus, path, c)).collect(),
            _ => vec![],
        },
        _ => vec![],
    }
}

enum Data {
    Items(Vec<(Tray, Vec<MenuEntry>)>),
}

fn main() {
    let (plugin, inputs) = Plugin::<Data>::start();
    let Ok(conn) = Connection::session() else {
        return;
    };
    let services: Arc<Mutex<Vec<String>>> = Arc::default();
    let (changed_tx, changed_rx) = std::sync::mpsc::channel::<()>();
    let own_watcher = !name_has_owner(&conn, WATCHER);
    let _server = if own_watcher {
        let w = Watcher { items: services.clone(), changed: changed_tx.clone() };
        let server = zbus::blocking::connection::Builder::session()
            .and_then(|b| b.name(WATCHER))
            .and_then(|b| b.serve_at(WATCHER_PATH, w))
            .and_then(|b| b.build());
        match server {
            Ok(c) => Some(c),
            Err(e) => {
                plugin.log(format!("couldn't be the tray watcher: {e}"));
                None
            }
        }
    } else {
        let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
        let _ = conn.request_name(host.as_str());
        let _ = conn.call_method(Some(WATCHER), WATCHER_PATH, Some(WATCHER), "RegisterStatusNotifierHost", &(host.as_str(),));
        None
    };

    // What to show: the watcher's list (ours or theirs), each item's props and menu.
    let send = plugin.data();
    let c = conn.clone();
    let list = services.clone();
    let trigger = nyx_plugin_sdk::services::refresher(Duration::from_secs(30), move || {
        let names: Vec<String> = if own_watcher {
            let mut l = list.lock().unwrap();
            l.retain(|s| name_has_owner(&c, &split(s).0));
            l.clone()
        } else {
            get_all(&c, WATCHER, WATCHER_PATH, WATCHER).map(|p| dbus::prop_strs(&p, "RegisteredStatusNotifierItems")).unwrap_or_default()
        };
        let items = names
            .iter()
            .filter_map(|s| read_item(&c, s))
            .map(|t| {
                let menu = t.menu.as_deref().map(|m| read_menu(&c, &t.bus, m)).unwrap_or_default();
                (t, menu)
            })
            .collect();
        send(Data::Items(items));
    });
    // Items register, change icons or menus, and go away: every signal on the bus
    // that could be one of those nudges a refresh.
    for path in ["/StatusNotifierItem", "/org/ayatana", WATCHER_PATH, "/MenuBar", "/org/freedesktop/DBus"] {
        dbus::listen(&conn, None, Some(path), trigger.clone());
    }
    let t2 = trigger.clone();
    std::thread::spawn(move || {
        for () in changed_rx {
            if t2.send(()).is_err() {
                return;
            }
        }
    });

    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut shown: Vec<String> = vec![];
    let mut current: HashMap<String, Tray> = HashMap::new();
    for input in inputs {
        match input {
            Input::Data(Data::Items(items)) => {
                let mut now = vec![];
                for (t, menu) in items {
                    let n = ids.len() + 1;
                    let n = *ids.entry(t.service.clone()).or_insert(n);
                    let id = format!("tray-{n}");
                    let node = match &t.icon {
                        Icon::Name(name) => icon(name),
                        Icon::File(f) => image_path(f).size(16, 16),
                        Icon::Png(data) => image_data(data.clone()).size(16, 16),
                        Icon::None => icon("application-x-executable-symbolic"),
                    };
                    plugin.item(item(&id).view(node).tooltip(t.title.clone()).hidden(t.passive));
                    if !menu.is_empty() {
                        plugin.menu(&id, menu);
                    }
                    current.insert(id.clone(), t);
                    now.push(id);
                }
                for gone in shown.iter().filter(|s| !now.contains(s)) {
                    plugin.remove(gone);
                    current.remove(gone);
                }
                shown = now;
            }
            Input::Event(e) => {
                let Some(t) = current.get(&e.item) else { continue };
                let (x, y) = e.anchor.as_ref().map(|a| (a.x + a.width / 2, a.y + a.height)).unwrap_or((0, 0));
                match (e.kind, e.button) {
                    (EventKind::Click, Some(1)) if !t.item_is_menu => {
                        let _ = conn.call_method(Some(t.bus.as_str()), t.path.as_str(), Some(ITEM_IFACE), "Activate", &(x, y));
                    }
                    (EventKind::Click, Some(2)) => {
                        let _ = conn.call_method(Some(t.bus.as_str()), t.path.as_str(), Some(ITEM_IFACE), "SecondaryActivate", &(x, y));
                    }
                    (EventKind::Click, _) => {
                        // No DBusMenu: let the app show its own.
                        let _ = conn.call_method(Some(t.bus.as_str()), t.path.as_str(), Some(ITEM_IFACE), "ContextMenu", &(x, y));
                    }
                    (EventKind::Action, _) => {
                        let Some(rest) = e.control.as_deref().and_then(|c| c.strip_prefix("menu:")) else { continue };
                        let mut parts = rest.splitn(3, '|');
                        let (Some(bus), Some(path), Some(id)) = (parts.next(), parts.next(), parts.next().and_then(|i| i.parse::<i32>().ok())) else {
                            continue;
                        };
                        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0);
                        let _ = conn.call_method(Some(bus), path, Some(MENU_IFACE), "Event", &(id, "clicked", Value::from(0i32), stamp));
                        let _ = trigger.send(());
                    }
                    _ => {}
                }
            }
            Input::Shutdown => break,
            _ => {}
        }
    }
}
