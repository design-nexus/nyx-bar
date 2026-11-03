//! Nyx Bar now playing: the current MPRIS player's track, its album art in the
//! popover with play/pause, previous, next and a seek slider.

use nyx_plugin_sdk::protocol::EventKind;
use nyx_plugin_sdk::services::mpris::{self, Player};
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin};
use std::path::PathBuf;
use std::time::Duration;

/// Album art as a local file: `file://` as it is, `https://` fetched once into the
/// plugin's own folder.
fn art_file(url: &str) -> Option<PathBuf> {
    if let Some(p) = url.strip_prefix("file://") {
        let p = PathBuf::from(percent_decode(p));
        return p.is_file().then_some(p);
    }
    if !url.starts_with("https://") {
        return None;
    }
    let dir = PathBuf::from(std::env::var("NYX_PLUGIN_STORAGE").ok()?).join("art");
    let _ = std::fs::create_dir_all(&dir);
    let mut h: u64 = 0xcbf29ce484222325;
    for b in url.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    let file = dir.join(format!("{h:x}"));
    if !file.is_file() {
        let ok = std::process::Command::new("curl")
            .args(["-fsSL", "--proto", "=https", "--max-time", "10", "--max-filesize", "4000000", "-o"])
            .arg(&file)
            .arg(url)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            let _ = std::fs::remove_file(&file);
            return None;
        }
    }
    Some(file)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
    t.push('…');
    t
}

fn clock(us: i64) -> String {
    let s = us / 1_000_000;
    format!("{}:{:02}", s / 60, s % 60)
}

fn show(plugin: &Plugin<Option<Player>>, p: &Option<Player>, max: usize) {
    let Some(p) = p else {
        plugin.item(item("media").hidden(true));
        plugin.popover("media", None);
        return;
    };
    let what = if p.artist.is_empty() { p.title.clone() } else { format!("{} — {}", p.artist, p.title) };
    let play_icon = if p.playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" };
    plugin.item(
        item("media")
            .view(hbox([icon(if p.playing { "media-playback-start-symbolic" } else { "media-playback-pause-symbolic" }), label(shorten(&what, max))]).spacing(6))
            .tooltip(format!("{what}\n{}", p.identity)),
    );
    let art = p.art.as_deref().and_then(art_file);
    let cover = match &art {
        Some(f) => image_path(&f.to_string_lossy()).size(220, 220),
        None => vbox([icon_sized("audio-x-generic-symbolic", 64)]).size(220, 120).background("field"),
    };
    let mut rows = vec![
        cover,
        markup(format!("<b>{}</b>", escape(&p.title))),
        label(if p.album.is_empty() { p.artist.clone() } else { format!("{} · {}", p.artist, p.album) }).color("dim_text"),
    ];
    if p.can_seek
        && let (Some(len), Some(pos)) = (p.length, p.position)
        && len > 0
    {
        rows.push(slider("seek", 0.0, len as f64 / 1e6, 1.0, pos as f64 / 1e6, None));
        rows.push(hbox([label(clock(pos)).color("dim_text"), label("").expand(), label(clock(len)).color("dim_text")]));
    }
    rows.push(
        hbox([
            button("previous", None, Some("media-skip-backward-symbolic")),
            button("play", None, Some(play_icon)),
            button("next", None, Some("media-skip-forward-symbolic")),
        ])
        .spacing(12),
    );
    plugin.popover("media", Some(vbox(rows).spacing(8).width(220)));
}

fn main() {
    let (plugin, inputs) = Plugin::<Option<Player>>::start();
    let mut max = plugin.setting_i64("max_chars", 32).clamp(8, 120) as usize;
    let send = plugin.data();
    mpris::watch(Duration::from_secs(5), move |p| send(p));
    let mut current: Option<Player> = None;
    for input in inputs {
        match input {
            Input::Data(p) => {
                show(&plugin, &p, max);
                current = p;
            }
            Input::Settings(s) => {
                max = s.get("max_chars").and_then(|v| v.as_i64()).unwrap_or(32).clamp(8, 120) as usize;
                show(&plugin, &current, max);
            }
            Input::Event(e) => {
                let Some(p) = &current else { continue };
                match (e.kind, e.control.as_deref()) {
                    (EventKind::Click, _) if e.button == Some(2) => mpris::play_pause(&p.bus),
                    (EventKind::Action, Some("play")) => mpris::play_pause(&p.bus),
                    (EventKind::Action, Some("next")) => mpris::next(&p.bus),
                    (EventKind::Action, Some("previous")) => mpris::previous(&p.bus),
                    (EventKind::Change, Some("seek")) => {
                        if let (Some(t), Some(v)) = (&p.track, e.value.as_ref().and_then(|v| v.as_f64())) {
                            mpris::set_position(&p.bus, t, (v * 1e6) as i64);
                        }
                    }
                    _ => {}
                }
            }
            Input::Shutdown => break,
            _ => {}
        }
    }
}
