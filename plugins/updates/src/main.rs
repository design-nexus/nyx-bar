//! Nyx Bar updates: a package icon and how many updates are waiting, hidden when there
//! are none. The popover lists them (old and new versions) with Update, which runs the
//! system's updater in a terminal. On Arch, from `checkupdates` every hour (or the
//! `interval` setting); elsewhere (NixOS updates through `nyx update`) it stays hidden.

use nyx_plugin_sdk::protocol::EventKind;
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin, have, spawn};
use serde_json::Value;
use std::time::Duration;

const ITEM: &str = "updates";
const SHOWN: usize = 12;

#[derive(Debug, Clone, PartialEq)]
struct Update {
    name: String,
    old: String,
    new: String,
}

/// `checkupdates`' lines: "name old -> new".
fn parse(out: &str) -> Vec<Update> {
    out.lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let (name, old, arrow, new) = (w.next()?, w.next()?, w.next()?, w.next()?);
            (arrow == "->").then(|| Update { name: name.into(), old: old.into(), new: new.into() })
        })
        .collect()
}

fn check() -> Option<Vec<Update>> {
    if !have("checkupdates") {
        return None;
    }
    let out = std::process::Command::new("checkupdates").stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    // 2 is "no updates"; anything else but 0 is a failed check, which keeps the last count.
    match out.status.code() {
        Some(0) => Some(parse(&String::from_utf8_lossy(&out.stdout))),
        Some(2) => Some(vec![]),
        _ => None,
    }
}

fn command(st: &Value) -> Vec<String> {
    let set: Vec<String> =
        st.get("command").and_then(|c| c.as_array()).map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default();
    if !set.is_empty() {
        set
    } else if have("omarchy-update") {
        vec!["omarchy-launch-floating-terminal-with-presentation".into(), "omarchy-update".into()]
    } else {
        ["xdg-terminal-exec", "sudo", "pacman", "-Syu"].map(String::from).to_vec()
    }
}

fn bar_item(u: &[Update]) -> Item {
    if u.is_empty() {
        return item(ITEM).hidden(true);
    }
    let n = u.len();
    item(ITEM)
        .view(hbox([icon_sized("software-update-available-symbolic|package-x-generic-symbolic", 15), label(n.to_string()).class("xs")]).spacing(4))
        .tooltip(if n == 1 { "1 update".to_string() } else { format!("{n} updates") })
}

fn popover(u: &[Update]) -> Node {
    let n = u.len();
    let count = if n == 1 { "1 update".to_string() } else { format!("{n} updates") };
    let mut rows = vec![
        hbox([
            icon_sized("software-update-available-symbolic|package-x-generic-symbolic", 16).class("dim"),
            label(count.clone()).class("uhead").expand(),
            button("refresh", None, Some("view-refresh-symbolic")).class("ibtn").tooltip("Check again"),
        ])
        .spacing(9)
        .margin(2, 4, 4, 4),
    ];
    let list: Vec<Node> = u
        .iter()
        .take(SHOWN)
        .map(|p| hbox([label(p.name.clone()).expand(), label(format!("{} → {}", p.old, p.new)).class("tertiary").class("small")]).spacing(10).class("urow"))
        .collect();
    rows.push(vbox(list).spacing(0));
    if n > SHOWN {
        rows.push(label(format!("and {} more", n - SHOWN)).class("tertiary").class("small").margin(2, 6, 0, 6));
    }
    rows.push(footer("update", "Update now", Some(label(count).class("tertiary").class("xs"))));
    vbox(rows).spacing(2).width(340)
}

fn main() {
    let (plugin, inputs) = Plugin::<Option<Vec<Update>>>::start();
    let mut cmd = command(plugin.settings());
    let every = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(plugin.setting_i64("interval", 60).clamp(5, 24 * 60) as u64));
    let send = plugin.data();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let ev = every.clone();
    std::thread::spawn(move || {
        loop {
            send(check());
            let _ = rx.recv_timeout(Duration::from_secs(ev.load(std::sync::atomic::Ordering::Relaxed) * 60));
        }
    });
    plugin.item(item(ITEM).hidden(true));
    let mut last: Vec<Update> = vec![];
    for input in inputs {
        match input {
            Input::Data(Some(u)) => {
                if u != last {
                    plugin.item(bar_item(&u));
                    plugin.popover(ITEM, (!u.is_empty()).then(|| popover(&u)));
                    last = u;
                }
            }
            Input::Settings(st) => {
                cmd = command(&st);
                every.store(st.get("interval").and_then(|v| v.as_i64()).unwrap_or(60).clamp(5, 24 * 60) as u64, std::sync::atomic::Ordering::Relaxed);
            }
            Input::Event(e) if e.kind == EventKind::Action => match e.control.as_deref() {
                Some("refresh") => {
                    let _ = tx.send(());
                }
                Some("update") => {
                    let argv: Vec<&str> = cmd.iter().map(String::as_str).collect();
                    spawn(&argv);
                }
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

    #[test]
    fn parses_checkupdates() {
        let u = parse("linux 6.17.1-1 -> 6.17.2-1\nmesa 1:25.2-1 -> 1:25.3-1\n\n");
        assert_eq!(u.len(), 2);
        assert_eq!(u[0], Update { name: "linux".into(), old: "6.17.1-1".into(), new: "6.17.2-1".into() });
        assert!(!bar_item(&[]).visible);
    }
}
