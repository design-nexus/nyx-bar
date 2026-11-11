//! Nyx Bar now playing: the track's art and title (with a level mark while it plays) in
//! the bar; the popover switches between players and has the art, seek, play controls,
//! the player's own volume and the sound output (Control Center's Sound page). Middle
//! click plays or pauses; scrolling changes the player's volume.
//!
//! As a desktop widget (`player`): album art, the track and the controls.

use nyx_plugin_sdk::protocol::{EventKind, Widget};
use nyx_plugin_sdk::services::dbus::{self, Connection};
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
        if b[i] == b'%' && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
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

const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

/// The player's own volume, 0 to 1, when it has one.
fn player_volume(conn: &Connection, bus: &str) -> Option<f64> {
    dbus::prop_num(&dbus::get_all(conn, bus, MPRIS_PATH, PLAYER_IFACE)?, "Volume")
}

fn set_volume(conn: &Connection, bus: &str, v: f64) {
    dbus::set(conn, bus, MPRIS_PATH, PLAYER_IFACE, "Volume", zbus::zvariant::Value::F64(v.clamp(0.0, 1.0)));
}

/// Four bars, a still level mark: the track is playing.
fn level() -> Node {
    let bar = |x: f64, h: f64| Draw::Rect { x, y: 10.0 - h, w: 2.0, h, radius: 1.0, fill: Some(color("accent")), stroke: None, width: None };
    vector([0.0, 0.0, 12.5, 10.0], vec![bar(0.0, 4.5), bar(3.5, 10.0), bar(7.0, 6.2), bar(10.5, 8.4)]).size(12, 10)
}

fn cover(p: &Player, size: i32) -> Node {
    match p.art.as_deref().and_then(art_file) {
        Some(f) => {
            let mut n = image_path(&f.to_string_lossy()).size(size, size);
            if let NodeKind::Image { radius, .. } = &mut n.kind {
                *radius = Some(if size > 40 { 10.0 } else { 4.0 });
            }
            n
        }
        None => centered(vbox([icon_sized("audio-x-generic-symbolic", (size / 2).max(12))])).size(size, size).class("card"),
    }
}

fn centered(mut n: Node) -> Node {
    if let NodeKind::Box { align, .. } = &mut n.kind {
        *align = Align::Center;
    }
    n
}

fn show(plugin: &Plugin<Option<Player>>, p: &Option<Player>, all: &[Player], volume: Option<f64>, max: usize, have_cc: bool) {
    let Some(p) = p else {
        plugin.item(item("media").hidden(true));
        plugin.popover("media", None);
        return;
    };
    let what = if p.artist.is_empty() { p.title.clone() } else { format!("{} — {}", p.artist, p.title) };
    let play_icon = if p.playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" };
    let mut inline = vec![cover(p, 18), label(shorten(&p.title, max)).class(if p.playing { "plain" } else { "dim" }).class("shrink")];
    inline.push(if p.playing { level() } else { icon_sized("media-playback-start-symbolic", 13).class("dim") });
    plugin.item(item("media").view(hbox(inline).spacing(7)).tooltip(format!("{what}\n{} · middle-click plays or pauses", p.identity)));

    let mut rows = vec![];
    if all.len() > 1 {
        let chips: Vec<Node> = all
            .iter()
            .map(|o| {
                let b = button(&format!("player:{}", o.bus), Some(&o.identity), None).class("chip");
                if o.bus == p.bus { b.class("selected") } else { b }
            })
            .collect();
        rows.push(hbox(chips).spacing(6));
    }
    let mut info = vec![label(&p.title).class("big")];
    if !p.artist.is_empty() {
        info.push(label(&p.artist).class("dim"));
    }
    if !p.album.is_empty() {
        info.push(label(&p.album).class("tertiary").class("small"));
    }
    info.push(label(&p.identity).class("tertiary").class("xs"));
    rows.push(hbox([cover(p, 96), vbox(info).spacing(2).expand()]).spacing(14));
    if p.can_seek
        && let (Some(len), Some(pos)) = (p.length, p.position)
        && len > 0
    {
        rows.push(hbox([label(clock(pos)).class("tertiary").class("xs"), slider("seek", 0.0, len as f64 / 1e6, 1.0, pos as f64 / 1e6, None).expand(), label(clock(len)).class("tertiary").class("xs")]).spacing(8));
    }
    rows.push(
        centered(hbox([
            button("previous", None, Some("media-skip-backward-symbolic")).class("flat"),
            button("play", None, Some(play_icon)).class("suggested-action").class("circular"),
            button("next", None, Some("media-skip-forward-symbolic")).class("flat"),
        ]))
        .spacing(18),
    );
    if let Some(v) = volume {
        rows.push(slider("player-volume", 0.0, 100.0, 1.0, (v * 100.0).round(), Some("audio-volume-high-symbolic")).tooltip(&format!("{}'s volume", p.identity)));
    }
    if have_cc {
        rows.push(button("output", Some("Sound output"), Some("audio-speakers-symbolic")).class("flat").tooltip("Choose where sound plays (Control Center)"));
    }
    plugin.popover("media", Some(vbox(rows).spacing(10).width(340)));
}

fn player_view(w: &Widget, p: &Player) -> Node {
    let large = w.size == "large";
    let art_size = if large { 168 } else { (w.height - 32).max(64) };
    let art = p.art.as_deref().and_then(art_file);
    let cover = match &art {
        Some(f) => image_path(&f.to_string_lossy()).size(art_size, art_size),
        None => vbox([icon_sized("audio-x-generic-symbolic", art_size / 3)]).size(art_size, art_size).background("field"),
    };
    let play_icon = if p.playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" };
    let controls = hbox([
        button("previous", None, Some("media-skip-backward-symbolic")),
        button("play", None, Some(play_icon)),
        button("next", None, Some("media-skip-forward-symbolic")),
    ])
    .spacing(8);
    let max = if large { 34 } else { 22 };
    let title = markup(format!("<b>{}</b>", escape(&shorten(&p.title, max))));
    let artist = label(shorten(if p.artist.is_empty() { &p.identity } else { &p.artist }, max)).color("dim_text");
    if large {
        vbox([cover, vbox([title, artist]).spacing(2), controls]).spacing(12)
    } else {
        hbox([cover, vbox([title, artist, label("").expand(), controls]).spacing(2).expand()]).spacing(14)
    }
}

fn show_player(plugin: &Plugin<Option<Player>>, widgets: &[Widget], p: &Option<Player>) {
    for w in widgets.iter().filter(|w| w.item == "player") {
        let it = match p {
            Some(p) => item("player").monitor(&w.monitor).view(player_view(w, p)).tooltip(format!("{} — {}", p.title, p.identity)),
            None => item("player").monitor(&w.monitor).hidden(true),
        };
        plugin.item(it);
    }
}

fn desktop(plugin: Plugin<Option<Player>>, inputs: std::sync::mpsc::Receiver<Input<Option<Player>>>) {
    mpris::watch(Duration::from_secs(5), plugin.data());
    let mut widgets = plugin.widgets().to_vec();
    let mut current: Option<Player> = None;
    show_player(&plugin, &widgets, &current);
    for input in inputs {
        match input {
            Input::Data(p) => {
                // Position ticks change nothing the widget shows.
                let same = |a: &Player, b: &Player| {
                    (&a.title, &a.artist, &a.art, a.playing, &a.identity) == (&b.title, &b.artist, &b.art, b.playing, &b.identity)
                };
                let changed = match (&current, &p) {
                    (Some(a), Some(b)) => !same(a, b),
                    (None, None) => false,
                    _ => true,
                };
                current = p;
                if changed {
                    show_player(&plugin, &widgets, &current);
                }
            }
            Input::Widgets(w) => {
                widgets = w;
                show_player(&plugin, &widgets, &current);
            }
            Input::Event(e) => {
                let Some(p) = &current else { continue };
                match (e.kind, e.control.as_deref()) {
                    (EventKind::Action, Some("play")) => mpris::play_pause(&p.bus),
                    (EventKind::Action, Some("next")) => mpris::next(&p.bus),
                    (EventKind::Action, Some("previous")) => mpris::previous(&p.bus),
                    _ => {}
                }
            }
            Input::Shutdown => break,
            _ => {}
        }
    }
}

fn main() {
    let (plugin, inputs) = Plugin::<Option<Player>>::start();
    if plugin.hello.kind == "desktop" {
        return desktop(plugin, inputs);
    }
    let mut max = plugin.setting_i64("max_chars", 22).clamp(8, 120) as usize;
    let send = plugin.data();
    mpris::watch(Duration::from_secs(5), send);
    let conn = Connection::session().ok();
    let have_cc = nyx_plugin_sdk::have("nyx-control-center");
    // The player the user picked in the popover; else the one playing.
    let mut chosen: Option<String> = None;
    let mut current: Option<Player> = None;
    let mut all: Vec<Player> = vec![];
    let refresh = |watched: &Option<Player>, chosen: &Option<String>, all: &mut Vec<Player>| -> (Option<Player>, Option<f64>) {
        *all = conn.as_ref().map(mpris::players).unwrap_or_default();
        all.retain(|p| !p.title.is_empty());
        let shown = chosen.as_ref().and_then(|b| all.iter().find(|p| &p.bus == b).cloned()).or_else(|| watched.clone());
        let volume = shown.as_ref().and_then(|p| conn.as_ref().and_then(|c| player_volume(c, &p.bus)));
        (shown, volume)
    };
    let mut watched: Option<Player> = None;
    for input in inputs {
        match input {
            Input::Data(p) => {
                watched = p;
                let (shown, volume) = refresh(&watched, &chosen, &mut all);
                show(&plugin, &shown, &all, volume, max, have_cc);
                current = shown;
            }
            Input::Settings(s) => {
                max = s.get("max_chars").and_then(|v| v.as_i64()).unwrap_or(22).clamp(8, 120) as usize;
                let (shown, volume) = refresh(&watched, &chosen, &mut all);
                show(&plugin, &shown, &all, volume, max, have_cc);
                current = shown;
            }
            Input::Event(e) => {
                if let (EventKind::Action, Some(bus)) = (e.kind, e.control.as_deref().and_then(|c| c.strip_prefix("player:"))) {
                    chosen = Some(bus.to_string());
                    let (shown, volume) = refresh(&watched, &chosen, &mut all);
                    show(&plugin, &shown, &all, volume, max, have_cc);
                    current = shown;
                    continue;
                }
                let Some(p) = &current else { continue };
                match (e.kind, e.control.as_deref()) {
                    (EventKind::Click, _) if e.button == Some(2) => mpris::play_pause(&p.bus),
                    (EventKind::Scroll, _) => {
                        if let (Some(c), Some(v)) = (&conn, conn.as_ref().and_then(|c| player_volume(c, &p.bus))) {
                            let step = if e.direction == Some(nyx_plugin_sdk::protocol::Direction::Up) { 0.05 } else { -0.05 };
                            set_volume(c, &p.bus, v + step);
                        }
                    }
                    (EventKind::Action, Some("play")) => mpris::play_pause(&p.bus),
                    (EventKind::Action, Some("next")) => mpris::next(&p.bus),
                    (EventKind::Action, Some("previous")) => mpris::previous(&p.bus),
                    (EventKind::Action, Some("output")) => nyx_plugin_sdk::spawn(&["nyx-control-center", "--toggle", "sound"]),
                    (EventKind::Change, Some("player-volume")) => {
                        if let (Some(c), Some(v)) = (&conn, e.value.as_ref().and_then(|v| v.as_f64())) {
                            set_volume(c, &p.bus, v / 100.0);
                        }
                    }
                    (EventKind::Change, Some("seek")) => {
                        if let (Some(t), Some(v)) = (&p.track, e.value.as_ref().and_then(|v| v.as_f64())) {
                            mpris::set_position(&p.bus, t, (v * 1e6) as i64);
                        }
                    }
                    (EventKind::PopoverOpen, _) => {
                        let (shown, volume) = refresh(&watched, &chosen, &mut all);
                        show(&plugin, &shown, &all, volume, max, have_cc);
                        current = shown;
                    }
                    _ => {}
                }
            }
            Input::Shutdown => break,
            _ => {}
        }
    }
}
