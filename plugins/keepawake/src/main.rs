//! Nyx Bar keep awake: a cup, in the accent colour while the computer is kept awake (no
//! screen lock, screen off or suspend). A click turns it on or off. The same switch as
//! Control Center's Keep awake: Omarchy's stay-awake indicator where Omarchy runs,
//! elsewhere the `nyx-keep-awake` user unit holding an idle inhibitor.

use nyx_plugin_sdk::protocol::EventKind;
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin, have};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

const ITEM: &str = "keepawake";
const UNIT: &str = "nyx-keep-awake.service";

fn omarchy() -> bool {
    have("omarchy-toggle-idle")
}

fn indicator() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"));
    state.join("omarchy/indicators/stay-awake")
}

fn quiet(argv: &[&str]) -> bool {
    Command::new(argv[0]).args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn awake() -> bool {
    if omarchy() { indicator().exists() } else { quiet(&["systemctl", "--user", "is-active", "--quiet", UNIT]) }
}

fn toggle(on: bool) {
    if omarchy() {
        quiet(&["omarchy-toggle-idle", "toggle"]);
    } else if on {
        quiet(&["systemctl", "--user", "stop", UNIT]);
    } else {
        quiet(&[
            "systemd-run",
            "--user",
            "--unit",
            UNIT,
            "--description",
            "Keep awake (Nyx Bar)",
            "systemd-inhibit",
            "--what=idle",
            "--who=Nyx Bar",
            "--why=Keep awake",
            "sleep",
            "infinity",
        ]);
    }
}

fn cup(on: bool) -> Node {
    let tint = || Some(color(if on { "accent" } else { "text" }));
    let line = |d: &str| Draw::Path { d: d.into(), fill: None, stroke: tint(), width: Some(1.8) };
    let mut draw = vec![
        Draw::Path {
            d: "M4 9H15.5V13.5C15.5 16.5 13.5 18.5 9.75 18.5C6 18.5 4 16.5 4 13.5Z".into(),
            fill: if on { tint() } else { None },
            stroke: tint(),
            width: Some(1.8),
        },
        line("M15.5 10.5C19 10.5 19 15 15.5 15"),
    ];
    if on {
        draw.push(line("M7.5 2.5C6.5 3.8 8.5 4.8 7.5 6.2"));
        draw.push(line("M11.5 2.5C10.5 3.8 12.5 4.8 11.5 6.2"));
    }
    vector([0.0, 0.0, 22.0, 22.0], draw).size(16, 16)
}

fn bar_item(on: bool) -> Item {
    let mut i = item(ITEM)
        .view(cup(on))
        .tooltip(if on { "Keeping awake: no screen lock, screen off or suspend\nClick to allow idle" } else { "Click to keep the computer awake" });
    if on {
        i = i.class("active");
    }
    i
}

fn main() {
    let (plugin, inputs) = Plugin::<bool>::start();
    let send = plugin.data();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        loop {
            send(awake());
            let _ = rx.recv_timeout(Duration::from_secs(5));
        }
    });
    let mut on = awake();
    plugin.item(bar_item(on));
    for input in inputs {
        match input {
            Input::Data(now) => {
                if now != on {
                    on = now;
                    plugin.item(bar_item(on));
                }
            }
            Input::Event(e) if e.kind == EventKind::Click && e.button.unwrap_or(1) == 1 => {
                toggle(on);
                on = awake();
                plugin.item(bar_item(on));
                let _ = tx.send(());
            }
            Input::Shutdown => break,
            _ => {}
        }
    }
}
