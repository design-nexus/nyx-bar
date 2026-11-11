//! Nyx Bar privacy: dots while an app records from the microphone (amber), uses the
//! camera (green) or shares the screen (violet), hidden otherwise. The popover says which
//! apps, since when, with Mute for the microphone and Stop for a screen share.
//!
//! PipeWire's graph (`pw-dump`, every three seconds): a running capture stream from an
//! app is the microphone; a running camera or screencast source is in use, by whatever
//! its links lead to. Apps that open `/dev/video*` themselves are found in `/proc`.

use nyx_plugin_sdk::protocol::EventKind;
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin, spawn};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

const ITEM: &str = "privacy";
const VIOLET: &str = "#a78bfa";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Kind {
    Mic,
    Camera,
    Screen,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Use {
    kind: Kind,
    app: String,
    /// The PipeWire node to stop, for a screen share.
    node: Option<u64>,
}

fn props(o: &Value) -> &Value {
    &o["info"]["props"]
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(|x| x.as_str()).filter(|s| !s.is_empty())
}

fn app_name(p: &Value) -> Option<String> {
    str_of(p, "application.name").or_else(|| str_of(p, "application.process.binary")).map(String::from)
}

fn truthy(p: &Value, k: &str) -> bool {
    matches!(p.get(k), Some(Value::Bool(true))) || str_of(p, k) == Some("true")
}

/// What PipeWire's graph says is in use.
fn from_graph(dump: &[Value], ignore: &[String]) -> Vec<Use> {
    let nodes: HashMap<u64, &Value> =
        dump.iter().filter(|o| o["type"] == "PipeWire:Interface:Node").filter_map(|o| Some((o["id"].as_u64()?, o))).collect();
    let consumers = |source: u64| -> Vec<String> {
        let mut out: Vec<String> = dump
            .iter()
            .filter(|o| o["type"] == "PipeWire:Interface:Link" && o["info"]["output-node-id"].as_u64() == Some(source))
            .filter_map(|l| nodes.get(&l["info"]["input-node-id"].as_u64()?))
            .filter_map(|n| app_name(props(n)))
            .collect();
        out.sort();
        out.dedup();
        out
    };
    let mut uses = vec![];
    for (&id, n) in &nodes {
        if n["info"]["state"] != "running" {
            continue;
        }
        let p = props(n);
        let class = str_of(p, "media.class").unwrap_or("");
        let name = str_of(p, "node.name").unwrap_or("");
        match class {
            "Stream/Input/Audio" => {
                let Some(app) = app_name(p) else { continue };
                let bin = str_of(p, "application.process.binary").unwrap_or("");
                if truthy(p, "stream.monitor") || truthy(p, "stream.capture.sink") || ignore.iter().any(|i| i == &app || i == bin) {
                    continue;
                }
                uses.push(Use { kind: Kind::Mic, app, node: None });
            }
            "Video/Source" if name.starts_with("xdph") || str_of(p, "application.name").is_some_and(|a| a.contains("portal")) => {
                let who = consumers(id);
                let app = if who.is_empty() { "An app".to_string() } else { who.join(", ") };
                uses.push(Use { kind: Kind::Screen, app, node: Some(id) });
            }
            "Video/Source" if str_of(p, "device.api") == Some("v4l2") || name.starts_with("v4l2_input") || name.starts_with("libcamera") => {
                for app in consumers(id) {
                    uses.push(Use { kind: Kind::Camera, app, node: None });
                }
            }
            _ => {}
        }
    }
    uses
}

/// Apps holding a camera device open themselves (not through PipeWire).
fn from_proc() -> Vec<Use> {
    let mut uses = vec![];
    let Ok(procs) = std::fs::read_dir("/proc") else { return uses };
    for p in procs.flatten() {
        let pid = p.file_name();
        if !pid.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(fds) = std::fs::read_dir(p.path().join("fd")) else { continue };
        let camera = fds.flatten().any(|fd| std::fs::read_link(fd.path()).is_ok_and(|t| t.to_string_lossy().starts_with("/dev/video")));
        if !camera {
            continue;
        }
        let comm = std::fs::read_to_string(p.path().join("comm")).unwrap_or_default().trim().to_string();
        if matches!(comm.as_str(), "pipewire" | "wireplumber" | "") {
            continue;
        }
        uses.push(Use { kind: Kind::Camera, app: comm, node: None });
    }
    uses
}

fn scan(ignore: &[String]) -> Vec<Use> {
    let out = std::process::Command::new("pw-dump").stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output();
    let dump: Vec<Value> = out.ok().and_then(|o| serde_json::from_slice(&o.stdout).ok()).unwrap_or_default();
    let mut uses = from_graph(&dump, ignore);
    uses.extend(from_proc());
    uses.sort();
    uses.dedup_by(|a, b| a.kind == b.kind && a.app.eq_ignore_ascii_case(&b.app));
    uses
}

fn tint(k: Kind) -> &'static str {
    match k {
        Kind::Mic => "warning",
        Kind::Camera => "success",
        Kind::Screen => VIOLET,
    }
}

fn dot(k: Kind) -> Node {
    vector([0.0, 0.0, 8.0, 8.0], vec![Draw::Circle { cx: 4.0, cy: 4.0, r: 4.0, fill: Some(color(tint(k))), stroke: None, width: None }]).size(8, 8)
}

fn bar_item(uses: &[Use]) -> Item {
    let mut kinds: Vec<Kind> = uses.iter().map(|u| u.kind).collect();
    kinds.dedup();
    if kinds.is_empty() {
        return item(ITEM).hidden(true);
    }
    let tip: Vec<String> = uses.iter().map(sentence).collect();
    item(ITEM).view(hbox(kinds.into_iter().map(dot)).spacing(4)).tooltip(tip.join("\n"))
}

fn sentence(u: &Use) -> String {
    match u.kind {
        Kind::Mic => format!("{} is using the microphone", u.app),
        Kind::Camera => format!("{} is using the camera", u.app),
        Kind::Screen => format!("{} is sharing your screen", u.app),
    }
}

fn popover(uses: &[Use], since: &HashMap<Use, String>, mic_muted: bool) -> Node {
    let mut rows = vec![
        hbox([icon_sized("security-high-symbolic", 16).class("accent"), label("In use now").class("strong").expand()]).spacing(8),
    ];
    for u in uses {
        let ic = match u.kind {
            Kind::Mic => "audio-input-microphone-symbolic",
            Kind::Camera => "camera-web-symbolic",
            Kind::Screen => "video-display-symbolic",
        };
        let mut row = vec![
            icon_sized(ic, 16).color(tint(u.kind)),
            vbox([
                label(sentence(u)).class("small"),
                label(since.get(u).map(|t| format!("Since {t}")).unwrap_or_default()).class("tertiary").class("xs"),
            ])
            .spacing(1)
            .expand(),
        ];
        match u.kind {
            Kind::Mic => row.push(button("mute", Some(if mic_muted { "Unmute" } else { "Mute" }), None)),
            Kind::Screen if u.node.is_some() => row.push(button(&format!("stop:{}", u.node.unwrap_or(0)), Some("Stop"), None)),
            _ => {}
        }
        rows.push(hbox(row).spacing(10).class("card"));
    }
    rows.push(button("settings", Some("Sound settings"), None).class("flat").class("link"));
    vbox(rows).spacing(8).width(340)
}

fn mic_muted() -> bool {
    std::process::Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SOURCE@"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("MUTED"))
}

fn ignore_list(st: &Value) -> Vec<String> {
    st.get("ignore").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default()
}

fn main() {
    let (plugin, inputs) = Plugin::<Vec<Use>>::start();
    let ignore = std::sync::Arc::new(std::sync::Mutex::new(ignore_list(plugin.settings())));
    let send = plugin.data();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let ig = ignore.clone();
    std::thread::spawn(move || {
        loop {
            let list = ig.lock().map(|l| l.clone()).unwrap_or_default();
            send(scan(&list));
            let _ = rx.recv_timeout(Duration::from_secs(3));
        }
    });
    plugin.item(item(ITEM).hidden(true));
    let mut last: Vec<Use> = vec![];
    let mut since: HashMap<Use, String> = HashMap::new();
    for input in inputs {
        match input {
            Input::Data(uses) => {
                since.retain(|u, _| uses.contains(u));
                for u in &uses {
                    since.entry(u.clone()).or_insert_with(|| chrono::Local::now().format("%H:%M").to_string());
                }
                if uses != last {
                    plugin.item(bar_item(&uses));
                }
                plugin.popover(ITEM, (!uses.is_empty()).then(|| popover(&uses, &since, mic_muted())));
                last = uses;
            }
            Input::Settings(st) => {
                if let Ok(mut l) = ignore.lock() {
                    *l = ignore_list(&st);
                }
                let _ = tx.send(());
            }
            Input::Event(e) if e.kind == EventKind::Action => {
                match e.control.as_deref() {
                    Some("mute") => spawn(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"]),
                    Some(c) if c.starts_with("stop:") => {
                        let id = &c[5..];
                        if id.bytes().all(|b| b.is_ascii_digit()) {
                            spawn(&["pw-cli", "destroy", id]);
                        }
                    }
                    Some("settings") => spawn(&["nyx-settings", "--section", "audio"]),
                    _ => {}
                }
                let _ = tx.send(());
            }
            Input::Shutdown => break,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: u64, class: &str, state: &str, props: Value) -> Value {
        let mut p = props;
        p["media.class"] = json!(class);
        json!({ "id": id, "type": "PipeWire:Interface:Node", "info": { "state": state, "props": p } })
    }

    #[test]
    fn reads_the_graph() {
        let dump = vec![
            node(1, "Stream/Input/Audio", "running", json!({ "application.name": "Chat" })),
            node(2, "Stream/Input/Audio", "running", json!({ "node.name": "echo-cancel-capture" })),
            node(3, "Stream/Input/Audio", "running", json!({ "application.name": "pavucontrol", "stream.monitor": true })),
            node(4, "Stream/Input/Audio", "idle", json!({ "application.name": "Recorder" })),
            node(10, "Video/Source", "running", json!({ "node.name": "xdph-streaming-1" })),
            node(11, "Stream/Input/Video", "running", json!({ "application.name": "OBS Studio" })),
            json!({ "id": 20, "type": "PipeWire:Interface:Link", "info": { "output-node-id": 10, "input-node-id": 11 } }),
            node(30, "Video/Source", "suspended", json!({ "device.api": "v4l2" })),
        ];
        let mut uses = from_graph(&dump, &[]);
        uses.sort();
        assert_eq!(
            uses,
            vec![
                Use { kind: Kind::Mic, app: "Chat".into(), node: None },
                Use { kind: Kind::Screen, app: "OBS Studio".into(), node: Some(10) },
            ]
        );
        assert!(from_graph(&dump, &["Chat".into()]).iter().all(|u| u.kind != Kind::Mic));
    }
}
