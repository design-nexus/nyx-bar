//! Nyx Bar weather: Nyx Weather's conditions in the bar (its icon and temperature, or
//! with the condition or today's range, and a chip for an alert), and a popover with
//! now and any alert, wind, humidity, UV and sunset, the next eight hours and five days
//! on one temperature scale. Read from `nyx-weather --json` every ten minutes; right
//! click opens Nyx Weather.

use nyx_plugin_sdk::protocol::EventKind;
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin, spawn};
use serde_json::Value;
use std::time::Duration;

const ITEM: &str = "now";

fn fetch() -> Option<Value> {
    let out = std::process::Command::new("nyx-weather").arg("--json").stdin(std::process::Stdio::null()).output().ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    v.get("temperature").is_some().then_some(v)
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

fn centered(mut n: Node) -> Node {
    if let NodeKind::Box { align, .. } = &mut n.kind {
        *align = Align::Center;
    }
    n
}

fn bar_item(w: &Value, look: &str) -> Item {
    let mut c = vec![icon(s(w, "icon")), label(s(w, "temperature"))];
    match look {
        "condition" => c.push(label(s(w, "condition")).class("dim")),
        "range" => c.push(label(format!("{} / {}", s(w, "high"), s(w, "low"))).class("tertiary").class("xs")),
        _ => {}
    }
    if let Some(alert) = w.get("alert").and_then(|a| a.as_str()) {
        let short: String = alert.split(':').nth(1).unwrap_or(alert).trim().chars().take(20).collect();
        c.push(hbox([icon_sized("dialog-warning-symbolic", 12), label(short)]).spacing(4).class("chip").class("warning"));
    }
    let mut tip = format!("{} · {}\nFeels like {} · H {} L {}", s(w, "place"), s(w, "condition"), s(w, "feels_like"), s(w, "high"), s(w, "low"));
    if let Some(a) = w.get("alert").and_then(|a| a.as_str()) {
        tip.push_str(&format!("\n⚠ {a}"));
    }
    tip.push_str("\nRight-click: open Weather");
    item(ITEM).view(hbox(c).spacing(6)).tooltip(tip).on_right_click(&["nyx-weather", "--toggle"])
}

fn chip(icon_name: &str, text: String) -> Node {
    hbox([icon_sized(icon_name, 12), label(text).class("small")]).spacing(5).class("chip")
}

fn popover(w: &Value) -> Node {
    let mut rows = vec![hbox([
        icon_sized("mark-location-symbolic", 12).class("tertiary"),
        label(format!("{} · updated {}", s(w, "location"), s(w, "updated"))).class("tertiary").class("small").expand(),
        button("refresh", None, Some("view-refresh-symbolic")).tooltip("Check again"),
    ])
    .spacing(6)];
    rows.push(
        hbox([
            icon_sized(s(w, "icon"), 44).class("warning"),
            label(s(w, "temperature")).class("huge"),
            vbox([label(s(w, "condition")).class("strong"), label(format!("Feels like {} · H {} L {}", s(w, "feels_like"), s(w, "high"), s(w, "low"))).class("dim").class("small")]).spacing(1).expand(),
        ])
        .spacing(14),
    );
    if let Some(a) = w.get("alert").and_then(|a| a.as_str()) {
        rows.push(hbox([icon("dialog-warning-symbolic").class("warning"), label(a).class("small").expand()]).spacing(10).class("card").class("alert"));
    }
    if let Some(r) = w.get("rain").and_then(|r| r.as_str()) {
        rows.push(hbox([icon("weather-showers-symbolic").class("accent"), label(r).class("small").expand()]).spacing(10).class("card"));
    }
    let mut chips = vec![chip("weather-windy-symbolic", s(w, "wind").to_string())];
    if let Some(h) = w.get("humidity").and_then(|h| h.as_i64()) {
        chips.push(chip("weather-showers-scattered-symbolic", format!("{h}%")));
    }
    if let Some(u) = w.get("uv").and_then(|u| u.as_i64()) {
        chips.push(chip("weather-clear-symbolic", format!("UV {u}")));
    }
    if !s(w, "sunset").is_empty() {
        chips.push(chip("daytime-sunset-symbolic|weather-clear-night-symbolic", s(w, "sunset").to_string()));
    }
    rows.push(hbox(chips).spacing(6));
    if let Some(hours) = w.get("hours").and_then(|h| h.as_array()).filter(|h| !h.is_empty()) {
        rows.push(label("NEXT HOURS").class("caps").margin(6, 0, 0, 2));
        let cols: Vec<Node> = hours
            .iter()
            .take(8)
            .map(|h| {
                let rain = h.get("precipitation").and_then(|p| p.as_i64()).unwrap_or(0);
                centered(vbox([
                    label(s(h, "time")).class("tertiary").class("xs"),
                    icon_sized(s(h, "icon"), 20).class("dim"),
                    label(s(h, "temperature")).class("strong").class("small"),
                    label(if rain >= 10 { format!("{rain}%") } else { String::new() }).class("accent").class("xs"),
                ]))
                .spacing(5)
                .width(44)
            })
            .collect();
        rows.push(hbox(cols).spacing(2));
    }
    if let Some(days) = w.get("days").and_then(|d| d.as_array()).filter(|d| !d.is_empty()) {
        rows.push(label("5 DAYS").class("caps").margin(6, 0, 0, 2));
        let num = |d: &Value, k: &str| d.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
        let lo = days.iter().map(|d| num(d, "low_c")).fold(f64::MAX, f64::min);
        let hi = days.iter().map(|d| num(d, "high_c")).fold(f64::MIN, f64::max);
        let span = (hi - lo).max(1.0);
        for d in days {
            let (a, b) = ((num(d, "low_c") - lo) / span * 120.0, (num(d, "high_c") - lo) / span * 120.0);
            let range = vector(
                [0.0, 0.0, 120.0, 6.0],
                vec![
                    Draw::Rect { x: 0.0, y: 0.0, w: 120.0, h: 6.0, radius: 3.0, fill: Some(color("track")), stroke: None, width: None },
                    Draw::Rect { x: a, y: 0.0, w: (b - a).max(6.0), h: 6.0, radius: 3.0, fill: Some(color("warning")), stroke: None, width: None },
                ],
            )
            .size(120, 6);
            rows.push(
                hbox([
                    label(s(d, "day")).class("strong").width(56),
                    icon_sized(s(d, "icon"), 18).class("dim").tooltip(s(d, "condition")),
                    label(s(d, "low")).class("tertiary").width(36),
                    centered(vbox([range])).expand(),
                    label(s(d, "high")).width(36),
                ])
                .spacing(8),
            );
        }
    }
    rows.push(button("open", Some("Open Weather"), None).class("flat").class("link"));
    vbox(rows).spacing(8).width(380)
}

fn main() {
    let (plugin, inputs) = Plugin::<Option<Value>>::start();
    let mut look = plugin.setting_str("look", "icon");
    let send = plugin.data();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        loop {
            send(fetch());
            // Every ten minutes, or at once when asked.
            let _ = rx.recv_timeout(Duration::from_secs(600));
        }
    });
    let mut last: Option<Value> = None;
    for input in inputs {
        match input {
            Input::Data(Some(w)) => {
                plugin.item(bar_item(&w, &look));
                plugin.popover(ITEM, Some(popover(&w)));
                last = Some(w);
            }
            Input::Data(None) => {
                if last.is_none() {
                    plugin.item(item(ITEM).hidden(true));
                }
            }
            Input::Settings(st) => {
                look = st.get("look").and_then(|v| v.as_str()).unwrap_or("icon").to_string();
                if let Some(w) = &last {
                    plugin.item(bar_item(w, &look));
                }
            }
            Input::Event(e) if e.kind == EventKind::Action => match e.control.as_deref() {
                Some("refresh") => {
                    let _ = tx.send(());
                }
                Some("open") => spawn(&["nyx-weather", "--toggle"]),
                _ => {}
            },
            Input::Shutdown => break,
            _ => {}
        }
    }
}
