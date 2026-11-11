//! Nyx Bar system status: network, Bluetooth, volume and battery, each its own item
//! (the user picks which), usually sharing one pill. Each opens its own Control Center
//! page (Wi-Fi, Bluetooth, Sound, Battery) and right-click opens the dashboard; the items
//! stay lit while Control Center is open. Scrolling over the volume changes it and a
//! middle click mutes. Without Control Center, a click opens a popover with quick
//! controls instead.

use nyx_plugin_sdk::protocol::{Direction, EventKind};
use nyx_plugin_sdk::services::audio::{self, Audio};
use nyx_plugin_sdk::services::dbus::{self, Connection};
use nyx_plugin_sdk::services::bluetooth::{self, Bluetooth};
use nyx_plugin_sdk::services::network::{self, Kind, Network};
use nyx_plugin_sdk::services::power::{self, Battery};
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin, have, spawn};

enum Data {
    Network(Network),
    Bluetooth(Bluetooth),
    Audio(Audio),
    Battery(Battery),
    ControlCenter(bool),
}

const CC: &str = "io.github.design_nexus.nyx.ControlCenter";
const PANEL_PATH: &str = "/io/github/design_nexus/nyx/Panel";
const PANEL_IFACE: &str = "io.github.design_nexus.nyx.Panel1";

/// Whether Control Center is installed (then it holds the controls) and open.
#[derive(Clone, Copy, Default)]
struct Cc {
    installed: bool,
    open: bool,
}

/// An item opens its Control Center page; right-click opens the dashboard.
fn with_cc(mut i: Item, page: &str, cc: Cc) -> Item {
    if !cc.installed {
        return i;
    }
    i.open = cc.open;
    i.on_click(&["nyx-control-center", "--toggle", page]).on_right_click(&["nyx-control-center", "--toggle"])
}

/// The quick-controls popover, only where Control Center isn't there to open.
fn popover(p: &Plugin<Data>, item: &str, view: Node, cc: Cc) {
    p.popover(item, (!cc.installed).then_some(view));
}

fn cc_open(conn: &Connection) -> bool {
    conn.call_method(Some(CC), PANEL_PATH, Some(PANEL_IFACE), "IsOpen", &()).ok().and_then(|r| r.body().deserialize::<bool>().ok()).unwrap_or(false)
}

fn network(p: &Plugin<Data>, n: &Network, cc: Cc) {
    let text = match (n.kind, &n.name) {
        (_, _) if n.missing => "NetworkManager isn't running".to_string(),
        (Kind::None, _) => "Not connected".to_string(),
        (_, Some(name)) => name.clone(),
        (_, None) => "Connected".to_string(),
    };
    let mut tip = text.clone();
    if let Some(s) = n.strength {
        tip.push_str(&format!(" · {s}%"));
    }
    if let Some(v) = &n.vpn {
        tip.push_str(&format!("\nVPN: {v}"));
    }
    let mut icons = vec![icon(n.icon())];
    if n.vpn.is_some() {
        icons.push(icon("network-vpn-symbolic"));
    }
    if cc.installed {
        tip.push_str("\nClick: Wi-Fi · right-click: Control Center");
    }
    p.item(with_cc(item("network").view(hbox(icons).spacing(2)).tooltip(tip), "wifi", cc));
    let mut rows = vec![header("Network")];
    if n.wifi_available {
        rows.push(toggle("wifi", "Wi-Fi", n.wifi_enabled));
    }
    let subtitle = match (n.kind, n.strength) {
        (Kind::Wifi, Some(s)) => Some(format!("Wi-Fi · signal {s}%")),
        (Kind::Ethernet, _) => Some("Wired".to_string()),
        (Kind::None, _) => None,
        _ => Some(if n.online { "Online".into() } else { "Limited".into() }),
    };
    rows.push(row(Some(n.icon()), text, subtitle, None));
    if let Some(v) = &n.vpn {
        rows.push(row(Some("network-vpn-symbolic"), v.clone(), Some("VPN".into()), None));
    }
    popover(p, "network", vbox(rows).spacing(8).width(260), cc);
}

fn bluetooth(p: &Plugin<Data>, b: &Bluetooth, cc: Cc) {
    if !b.available {
        p.item(item("bluetooth").hidden(true));
        return;
    }
    let tip = match (b.powered, b.connected.len()) {
        (false, _) => "Bluetooth is off".to_string(),
        (true, 0) => "Bluetooth is on".to_string(),
        (true, _) => b.connected.iter().map(|d| match d.battery {
            Some(bat) => format!("{} · {bat}%", d.name),
            None => d.name.clone(),
        }).collect::<Vec<_>>().join("\n"),
    };
    p.item(with_cc(item("bluetooth").view(icon(b.icon())).tooltip(tip), "bluetooth", cc));
    let mut rows = vec![header("Bluetooth"), toggle("bt-power", "Bluetooth", b.powered)];
    for d in &b.connected {
        rows.push(row(
            Some(d.icon.as_deref().map(|i| format!("{i}-symbolic")).as_deref().unwrap_or("bluetooth-symbolic")),
            d.name.clone(),
            Some(match d.battery {
                Some(bat) => format!("Connected · battery {bat}%"),
                None => "Connected".into(),
            }),
            Some(&format!("disconnect:{}", d.path)),
        ));
    }
    if b.powered && b.connected.is_empty() {
        rows.push(label("No devices connected").color("dim_text"));
    }
    popover(p, "bluetooth", vbox(rows).spacing(8).width(260), cc);
}

fn volume(p: &Plugin<Data>, a: &Audio, show_pct: bool, cc: Cc) {
    if !a.available {
        p.item(item("volume").hidden(true));
        return;
    }
    let pct = a.percent();
    let mut parts = vec![icon(a.icon())];
    if show_pct {
        parts.push(label(format!("{pct}%")).id("pct"));
    }
    let tip = if a.muted { format!("Muted ({pct}%)") } else { format!("Volume {pct}%") };
    let tip = format!("{tip}\nScroll to change it · middle-click mutes");
    let i = item("volume").view(hbox(parts).spacing(4)).tooltip(tip).on_middle_click(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]);
    p.item(if cc.installed { with_cc(i, "sound", cc) } else { i.on_right_click(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]) });
    let mut rows = vec![header("Sound"), slider("volume", 0.0, 150.0, 1.0, pct as f64, Some(a.icon())), toggle("mute", "Mute", a.muted)];
    if let Some(m) = a.mic_muted {
        rows.push(toggle("mic", "Mute microphone", m));
    }
    popover(p, "volume", vbox(rows).spacing(8).width(280), cc);
}

fn battery(p: &Plugin<Data>, b: &Battery, show_pct: bool, cc: Cc) {
    if !b.present {
        p.item(item("battery").hidden(true));
        return;
    }
    let pct = b.percent.round() as u32;
    let mut parts = vec![icon(&b.icon())];
    if show_pct {
        parts.push(label(format!("{pct}%")));
    }
    let state = if b.full {
        "Fully charged".to_string()
    } else if b.charging {
        match b.time_left {
            Some(t) => format!("Charging · full in {}", power::duration(t)),
            None => "Charging".into(),
        }
    } else {
        match b.time_left {
            Some(t) => format!("{} left", power::duration(t)),
            None => "On battery".into(),
        }
    };
    let mut i = item("battery").view(hbox(parts).spacing(4)).tooltip(format!("{pct}% · {state}"));
    if !b.charging && pct <= 10 {
        i = i.class("danger");
    } else if !b.charging && pct <= 20 {
        i = i.class("warning");
    }
    p.item(with_cc(i, "battery", cc));
    let mut rows = vec![header(&format!("Battery {pct}%")), progress(b.percent / 100.0), label(state).color("dim_text")];
    if let Some(profile) = &b.profile {
        rows.push(separator());
        for (id, name) in [("power-saver", "Power saver"), ("balanced", "Balanced"), ("performance", "Performance")] {
            let mark = if profile == id { Some("object-select-symbolic") } else { None };
            rows.push(row(mark, name, None, Some(&format!("profile:{id}"))));
        }
    }
    popover(p, "battery", vbox(rows).spacing(8).width(260), cc);
}

fn main() {
    let (plugin, inputs) = Plugin::<Data>::start();
    let send = plugin.data();
    // The items in the pill's order, before any of them knows its state.
    for id in ["network", "bluetooth", "volume", "battery"] {
        if plugin.enabled(id) {
            plugin.item(item(id).hidden(true));
        }
    }
    let mut cc = Cc { installed: have("nyx-control-center"), open: false };
    if cc.installed
        && let Ok(conn) = Connection::session()
    {
        let send = send.clone();
        let c = conn.clone();
        // Opened and Closed say when it changes; asking now and then catches the rest.
        let trigger = nyx_plugin_sdk::services::refresher(std::time::Duration::from_secs(30), move || send(Data::ControlCenter(cc_open(&c))));
        dbus::listen(&conn, None, Some(PANEL_PATH), trigger.clone());
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(700));
                if trigger.send(()).is_err() {
                    return;
                }
            }
        });
    }
    if plugin.enabled("network") {
        let s = send.clone();
        network::watch(move |n| s(Data::Network(n)));
    }
    if plugin.enabled("bluetooth") {
        let s = send.clone();
        bluetooth::watch(move |b| s(Data::Bluetooth(b)));
    }
    if plugin.enabled("volume") {
        let s = send.clone();
        audio::watch(move |a| s(Data::Audio(a)));
    }
    if plugin.enabled("battery") {
        let s = send.clone();
        power::watch(move |b| s(Data::Battery(b)));
    }
    let mut volume_pct = plugin.setting_bool("volume_percent", false);
    let mut battery_pct = plugin.setting_bool("battery_percent", true);
    let (mut last_net, mut last_bt, mut last_audio, mut last_battery) = (None::<Network>, None::<Bluetooth>, None::<Audio>, None::<Battery>);
    for input in inputs {
        let redraw_all = |cc: Cc, vp: bool, bp: bool, n: &Option<Network>, b: &Option<Bluetooth>, a: &Option<Audio>, bat: &Option<Battery>| {
            if let Some(n) = n {
                network(&plugin, n, cc);
            }
            if let Some(b) = b {
                bluetooth(&plugin, b, cc);
            }
            if let Some(a) = a {
                volume(&plugin, a, vp, cc);
            }
            if let Some(x) = bat {
                battery(&plugin, x, bp, cc);
            }
        };
        match input {
            Input::Data(Data::Network(n)) => {
                network(&plugin, &n, cc);
                last_net = Some(n);
            }
            Input::Data(Data::Bluetooth(b)) => {
                bluetooth(&plugin, &b, cc);
                last_bt = Some(b);
            }
            Input::Data(Data::Audio(a)) => {
                volume(&plugin, &a, volume_pct, cc);
                last_audio = Some(a);
            }
            Input::Data(Data::Battery(b)) => {
                battery(&plugin, &b, battery_pct, cc);
                last_battery = Some(b);
            }
            Input::Data(Data::ControlCenter(open)) => {
                if open != cc.open {
                    cc.open = open;
                    redraw_all(cc, volume_pct, battery_pct, &last_net, &last_bt, &last_audio, &last_battery);
                }
            }
            Input::Settings(s) => {
                volume_pct = s.get("volume_percent").and_then(|v| v.as_bool()).unwrap_or(false);
                battery_pct = s.get("battery_percent").and_then(|v| v.as_bool()).unwrap_or(true);
                redraw_all(cc, volume_pct, battery_pct, &last_net, &last_bt, &last_audio, &last_battery);
            }
            Input::Event(e) => match (e.item.as_str(), e.kind) {
                ("volume", EventKind::Scroll) => {
                    audio::change(if e.direction == Some(Direction::Up) { 5 } else { -5 });
                }
                ("volume", EventKind::Change) if e.control.as_deref() == Some("volume") => {
                    if let Some(v) = e.value.as_ref().and_then(|v| v.as_f64()) {
                        audio::set(v / 100.0);
                    }
                }
                (_, EventKind::Toggle) => match e.control.as_deref() {
                    Some("wifi") => network::set_wifi(e.value.as_ref().and_then(|v| v.as_bool()).unwrap_or(true)),
                    Some("mute") => audio::toggle_mute(),
                    Some("mic") => audio::toggle_mic(),
                    Some("bt-power") => {
                        if let Some(a) = last_bt.as_ref().and_then(|b| b.adapter.clone()) {
                            bluetooth::set_powered(&a, e.value.as_ref().and_then(|v| v.as_bool()).unwrap_or(true));
                        }
                    }
                    _ => {}
                },
                (_, EventKind::Action) => {
                    let c = e.control.unwrap_or_default();
                    if let Some(page) = c.strip_prefix("cc:") {
                        let mut argv = vec!["nyx-control-center", "--toggle"];
                        if !page.is_empty() {
                            argv.push(page);
                        }
                        spawn(&argv);
                    } else if let Some(profile) = c.strip_prefix("profile:") {
                        power::set_profile(profile);
                    } else if let Some(dev) = c.strip_prefix("disconnect:") {
                        bluetooth::disconnect(dev);
                    }
                }
                _ => {}
            },
            Input::Shutdown => break,
            _ => {}
        }
    }
}
