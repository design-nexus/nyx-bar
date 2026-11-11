//! Nyx Bar recording: while Nyx Capture records the screen, a red dot, the time since
//! it started and a stop button; hidden otherwise. A click stops the recording (Capture
//! saves it), right click pauses or resumes it. Nyx Capture keeps the recording in
//! `$XDG_RUNTIME_DIR/nyx-capture/recording.json`, read every second.

use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin};
use serde_json::Value;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const ITEM: &str = "recording";

#[derive(Debug, Clone, PartialEq)]
struct Rec {
    secs: u64,
    paused: bool,
}

fn session_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join("nyx-capture").join("recording.json")
}

fn read() -> Option<Rec> {
    let path = session_path();
    let s: Value = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    let pid = s.get("pid")?.as_u64()?;
    if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
        return None;
    }
    // When the recording file was made; the session file is rewritten on pause.
    let started = s
        .get("path")
        .and_then(|p| p.as_str())
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.created().ok())
        .or_else(|| std::fs::metadata(&path).ok()?.modified().ok())?;
    let secs = SystemTime::now().duration_since(started).map(|d| d.as_secs()).unwrap_or(0);
    Some(Rec { secs, paused: s.get("paused").and_then(|p| p.as_bool()).unwrap_or(false) })
}

fn clock(secs: u64) -> String {
    if secs >= 3600 { format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60) } else { format!("{:02}:{:02}", secs / 60, secs % 60) }
}

fn bar_item(r: Option<&Rec>) -> Item {
    let Some(r) = r else { return item(ITEM).hidden(true) };
    let dot = vector([0.0, 0.0, 8.0, 8.0], vec![Draw::Circle { cx: 4.0, cy: 4.0, r: 4.0, fill: Some(color("danger")), stroke: None, width: None }]).size(8, 8);
    let time = if r.paused { "Paused".to_string() } else { clock(r.secs) };
    item(ITEM)
        .view(hbox([dot, label(time).class("tnum"), icon_sized("media-playback-stop-symbolic", 14)]).spacing(6))
        .class("recording")
        .tooltip(if r.paused { "Recording paused\nClick: stop · Right-click: resume" } else { "Recording the screen\nClick: stop · Right-click: pause" })
        .on_click(&["nyx-capture", "--record", "--stop"])
        .on_right_click(&["nyx-capture", "--record", "--pause"])
}

fn main() {
    let (plugin, inputs) = Plugin::<Option<Rec>>::start();
    let send = plugin.data();
    std::thread::spawn(move || {
        loop {
            send(read());
            std::thread::sleep(Duration::from_secs(1));
        }
    });
    plugin.item(item(ITEM).hidden(true));
    let mut last: Option<Rec> = None;
    for input in inputs {
        match input {
            Input::Data(r) => {
                if r != last {
                    plugin.item(bar_item(r.as_ref()));
                    last = r;
                }
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
    fn clocks() {
        assert_eq!(clock(134), "02:14");
        assert_eq!(clock(3725), "1:02:05");
    }
}
