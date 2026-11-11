//! Nyx Bar panels: the Notification Center's bell (with the unread count, a moon in Do
//! not disturb, and a right-click menu: Do not disturb for an hour, until morning or
//! until turned off, clear, settings) and the Control Center's button (lit while it's
//! open; off by default, as the status icons open Control Center). Each shows only when
//! its program is installed.

use nyx_plugin_sdk::protocol::EventKind;
use nyx_plugin_sdk::services::dbus::{self, Connection};
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin, have, spawn};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

/// The bell with the unread count beside it, or a moon in Do not disturb.
fn bell(unread: u64, dnd: bool) -> Node {
    if dnd {
        return icon("weather-clear-night-symbolic|night-light-symbolic").class("dnd");
    }
    let mut c = vec![icon("notifications-symbolic|preferences-system-notifications-symbolic")];
    if unread > 0 {
        c.push(badge(if unread > 99 { "99+".to_string() } else { unread.to_string() }));
    }
    hbox(c).spacing(3)
}

/// Do not disturb that turns itself off: the time it ends, kept across restarts.
fn dnd_until_file() -> PathBuf {
    state_home().join("nyx/dnd-until")
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// The next 07:00 local time, as `date` says it.
fn tomorrow_morning() -> i64 {
    std::process::Command::new("date")
        .args(["-d", "tomorrow 07:00", "+%s"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or_else(|| now_secs() + 8 * 3600)
}

fn set_dnd(conn: Option<&Connection>, on: bool) {
    let set = conn.is_some_and(|c| c.call_method(Some(NOTIFYD), NOTIFYD_PATH, Some(NOTIFYD_IFACE), "SetDnd", &(on,)).is_ok());
    if !set && have("omarchy-toggle-notification-silencing") && dnd(conn) != on {
        spawn(&["omarchy-toggle-notification-silencing"]);
    }
}

fn menu(p: &Plugin<Data>, dnd_on: bool, unread: u64) {
    let with_icon = |mut e: MenuEntry, icon: &str| {
        e.icon = Some(icon.into());
        e
    };
    let mut entries = vec![];
    if dnd_on {
        entries.push(with_icon(menu_entry("dnd:off", "Turn off Do not disturb"), "preferences-system-notifications-symbolic|notifications-symbolic"));
    }
    let mut dnd_menu = with_icon(menu_entry("dnd", "Do not disturb"), "weather-clear-night-symbolic");
    dnd_menu.children = vec![menu_entry("dnd:1h", "For 1 hour"), menu_entry("dnd:morning", "Until tomorrow morning"), menu_entry("dnd:on", "Until I turn it off")];
    entries.push(dnd_menu);
    entries.push(MenuEntry { separator: true, ..Default::default() });
    let mut clear = with_icon(menu_entry("clear", &if unread > 0 { format!("Clear {unread} notifications") } else { "Clear notifications".into() }), "window-close-symbolic");
    clear.enabled = unread > 0;
    entries.push(clear);
    entries.push(with_icon(menu_entry("settings", "Notification settings"), "emblem-system-symbolic|preferences-system-symbolic"));
    p.menu("notifications", entries);
}

fn grid(open: bool) -> Node {
    let cell = |x: f64, y: f64| Draw::Rect {
        x,
        y,
        w: 4.8,
        h: 4.8,
        radius: 1.5,
        fill: open.then(|| color("accent")),
        stroke: Some(color(if open { "accent" } else { "text" })),
        width: Some(1.25),
    };
    vector([0.0, 0.0, 16.0, 16.0], vec![cell(2.4, 2.4), cell(8.8, 2.4), cell(2.4, 8.8), cell(8.8, 8.8)]).size(16, 16)
}

fn main() {
    let (plugin, inputs) = Plugin::<Data>::start();
    let session = Connection::session().ok();
    let have_nc = have("nyx-notification-center");
    let have_cc = have("nyx-control-center");

    if plugin.enabled("notifications") && have_nc {
        let send = plugin.data();
        let conn = session.clone();
        // A timed Do not disturb ends here, even after a restart.
        let c2 = session.clone();
        std::thread::spawn(move || {
            loop {
                if let Some(until) = std::fs::read_to_string(dnd_until_file()).ok().and_then(|t| t.trim().parse::<i64>().ok())
                    && now_secs() >= until
                {
                    let _ = std::fs::remove_file(dnd_until_file());
                    set_dnd(c2.as_ref(), false);
                }
                std::thread::sleep(Duration::from_secs(20));
            }
        });
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
    let (mut unread_now, mut dnd_now) = (0u64, false);
    for input in inputs {
        match input {
            Input::Data(Data::Notifications { unread, dnd }) => {
                let until = std::fs::read_to_string(dnd_until_file()).ok().and_then(|t| t.trim().parse::<i64>().ok());
                let tip = match (dnd, unread) {
                    (true, _) => match until.filter(|_| dnd) {
                        Some(u) => format!("Do not disturb, for {} more min", ((u - now_secs()) / 60).max(1)),
                        None => "Do not disturb is on".to_string(),
                    },
                    (false, 0) => "No new notifications".to_string(),
                    (false, 1) => "1 new notification".to_string(),
                    (false, n) => format!("{n} new notifications"),
                };
                plugin.item(
                    item("notifications")
                        .view(bell(unread, dnd))
                        .tooltip(format!("{tip}\nRight-click: Do not disturb, clear"))
                        .on_click(&["nyx-notification-center", "--toggle"]),
                );
                if (unread, dnd) != (unread_now, dnd_now) || unread == 0 {
                    menu(&plugin, dnd, unread);
                }
                (unread_now, dnd_now) = (unread, dnd);
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
            Input::Event(e) if e.item == "notifications" && e.kind == EventKind::Action => {
                let until = |t: i64| {
                    let _ = std::fs::create_dir_all(state_home().join("nyx"));
                    let _ = std::fs::write(dnd_until_file(), t.to_string());
                };
                match e.control.as_deref() {
                    Some("dnd:off") => {
                        let _ = std::fs::remove_file(dnd_until_file());
                        set_dnd(session.as_ref(), false);
                    }
                    Some("dnd:on") => {
                        let _ = std::fs::remove_file(dnd_until_file());
                        set_dnd(session.as_ref(), true);
                    }
                    Some("dnd:1h") => {
                        until(now_secs() + 3600);
                        set_dnd(session.as_ref(), true);
                    }
                    Some("dnd:morning") => {
                        until(tomorrow_morning());
                        set_dnd(session.as_ref(), true);
                    }
                    Some("clear") => {
                        if let Some(c) = &session {
                            let _ = c.call_method(Some(NOTIFYD), NOTIFYD_PATH, Some(NOTIFYD_IFACE), "DismissAll", &());
                        }
                    }
                    Some("settings") => spawn(&["nyx-settings", "--section", "notifications"]),
                    _ => {}
                }
            }
            Input::Shutdown => break,
            _ => {}
        }
    }
}
