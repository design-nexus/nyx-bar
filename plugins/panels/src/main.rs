//! Nyx Bar panels: the Notification Center's bell (a dot for unread notifications,
//! crossed out in Do not disturb) and the Control Center's button (lit while it's
//! open). Each shows only when its program is installed.

use nyx_plugin_sdk::protocol::EventKind;
use nyx_plugin_sdk::services::dbus::{self, Connection};
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin, have, spawn};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const NOTIFYD: &str = "io.github.design_nexus.nyx.Notifications";
const NOTIFYD_PATH: &str = "/io/github/design_nexus/nyx/Notifications";
const NOTIFYD_IFACE: &str = "io.github.design_nexus.nyx.Notifications1";
const CC: &str = "io.github.design_nexus.nyx.ControlCenter";
const PANEL_PATH: &str = "/io/github/design_nexus/nyx/Panel";
const PANEL_IFACE: &str = "io.github.design_nexus.nyx.Panel1";

enum Data {
    Notifications { unread: u64, dnd: bool },
    ControlCenter(bool),
}

fn state_home() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state"))
}

fn json(path: &PathBuf) -> serde_json::Value {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn dnd(conn: Option<&Connection>) -> bool {
    if let Some(c) = conn
        && let Ok(r) = c.call_method(Some(NOTIFYD), NOTIFYD_PATH, Some(NOTIFYD_IFACE), "Dnd", &())
        && let Ok(on) = r.body().deserialize::<bool>()
    {
        return on;
    }
    json(&state_home().join("omarchy/notifications/quiet.json"))["dnd"].as_bool().unwrap_or(false)
}

fn cc_open(conn: &Connection) -> bool {
    conn.call_method(Some(CC), PANEL_PATH, Some(PANEL_IFACE), "IsOpen", &()).ok().and_then(|r| r.body().deserialize::<bool>().ok()).unwrap_or(false)
}

fn bell(unread: bool, dnd: bool) -> Node {
    let mut c = vec![
        Draw::Path { d: "M8 1.6c-2.4 0-3.9 1.9-3.9 4.2v2.6L2.6 10.6v.9h10.8v-.9L11.9 8.4V5.8c0-2.3-1.5-4.2-3.9-4.2z".into(), fill: Some(color("text")), stroke: None, width: None },
        Draw::Circle { cx: 8.0, cy: 13.3, r: 1.4, fill: Some(color("text")), stroke: None, width: None },
    ];
    if dnd {
        c.push(Draw::Line { x1: 2.0, y1: 2.0, x2: 14.0, y2: 14.0, stroke: Some(color("panel")), width: Some(3.2), round: true });
        c.push(Draw::Line { x1: 2.0, y1: 2.0, x2: 14.0, y2: 14.0, stroke: Some(color("text")), width: Some(1.4), round: true });
    } else if unread {
        c.push(Draw::Circle { cx: 12.6, cy: 3.4, r: 3.0, fill: Some(color("accent")), stroke: Some(color("panel")), width: Some(1.5) });
    }
    vector([0.0, 0.0, 16.0, 16.0], c).size(16, 16)
}

fn grid(open: bool) -> Node {
    let cell = |x: f64, y: f64| Draw::Rect {
        x,
        y,
        w: 5.6,
        h: 5.6,
        radius: 1.8,
        fill: open.then(|| color("accent")),
        stroke: Some(color(if open { "accent" } else { "text" })),
        width: Some(1.4),
    };
    vector([0.0, 0.0, 16.0, 16.0], vec![cell(1.6, 1.6), cell(8.8, 1.6), cell(1.6, 8.8), cell(8.8, 8.8)]).size(16, 16)
}

fn main() {
    let (plugin, inputs) = Plugin::<Data>::start();
    let session = Connection::session().ok();
    let have_nc = have("nyx-notification-center");
    let have_cc = have("nyx-control-center");

    if plugin.enabled("notifications") && have_nc {
        let send = plugin.data();
        let conn = session.clone();
        let file = state_home().join("nyx/notifications.json");
        let quiet = state_home().join("omarchy/notifications/quiet.json");
        let mtime = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let mut last: (Option<SystemTime>, Option<SystemTime>) = (None, None);
        // notifyd's Changed signal, and the files it writes (watched every 2 s).
        let trigger = nyx_plugin_sdk::services::refresher(Duration::from_secs(30), move || {
            let unread = json(&file)["unread"].as_u64().unwrap_or(0);
            send(Data::Notifications { unread, dnd: dnd(conn.as_ref()) });
        });
        if let Some(c) = &session {
            dbus::listen(c, None, Some(NOTIFYD_PATH), trigger.clone());
        }
        let (file, quiet) = (state_home().join("nyx/notifications.json"), quiet);
        std::thread::spawn(move || {
            loop {
                let now = (mtime(&file), mtime(&quiet));
                if now != last {
                    last = now;
                    if trigger.send(()).is_err() {
                        return;
                    }
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    }
    if plugin.enabled("control-center") && have_cc {
        plugin.item(item("control-center").view(grid(false)).tooltip("Control Center").on_click(&["nyx-control-center", "--toggle"]));
        if let Some(c) = &session {
            let send = plugin.data();
            let conn = c.clone();
            // Opened says when it opens; while it's open, ask now and then whether it still is.
            let trigger = nyx_plugin_sdk::services::refresher(Duration::from_secs(30), move || send(Data::ControlCenter(cc_open(&conn))));
            dbus::listen(c, None, Some(PANEL_PATH), trigger.clone());
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_millis(700));
                    if trigger.send(()).is_err() {
                        return;
                    }
                }
            });
        }
    }
    let mut cc_was = false;
    for input in inputs {
        match input {
            Input::Data(Data::Notifications { unread, dnd }) => {
                let tip = match (dnd, unread) {
                    (true, _) => "Do not disturb is on".to_string(),
                    (false, 0) => "No new notifications".to_string(),
                    (false, 1) => "1 new notification".to_string(),
                    (false, n) => format!("{n} new notifications"),
                };
                plugin.item(
                    item("notifications")
                        .view(bell(unread > 0, dnd))
                        .tooltip(format!("{tip}\nRight-click: Do not disturb"))
                        .on_click(&["nyx-notification-center", "--toggle"]),
                );
            }
            Input::Data(Data::ControlCenter(open)) => {
                if open != cc_was {
                    cc_was = open;
                    let mut i = item("control-center").view(grid(open)).tooltip("Control Center").on_click(&["nyx-control-center", "--toggle"]);
                    if open {
                        i = i.class("active");
                    }
                    plugin.item(i);
                }
            }
            Input::Event(e) if e.item == "notifications" && e.kind == EventKind::Click && e.button == Some(3) => {
                let toggled = session
                    .as_ref()
                    .is_some_and(|c| c.call_method(Some(NOTIFYD), NOTIFYD_PATH, Some(NOTIFYD_IFACE), "ToggleDnd", &()).is_ok());
                if !toggled && have("omarchy-toggle-notification-silencing") {
                    spawn(&["omarchy-toggle-notification-silencing"]);
                }
            }
            Input::Shutdown => break,
            _ => {}
        }
    }
}
