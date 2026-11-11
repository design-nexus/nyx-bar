//! Nyx Bar stats: CPU, memory, the hottest CPU sensor and, when a GPU reports it, its
//! load. Sampled every few seconds, and only while the bar is visible.
//!
//! As a desktop widget (`graphs`): the last ten minutes as graphs, sampled all along
//! and drawn only while the widget can be seen.

use nyx_plugin_sdk::protocol::Widget;
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin, have};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

const HISTORY: usize = 100;

#[derive(Default)]
struct Sampler {
    last_cpu: Option<(u64, u64)>,
    temp: Option<PathBuf>,
    gpu: Option<PathBuf>,
    fan: Option<PathBuf>,
}

fn read(p: &str) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

impl Sampler {
    fn new() -> Sampler {
        let mut s = Sampler::default();
        // The CPU package sensor: coretemp, k10temp or zenpower.
        if let Ok(dirs) = std::fs::read_dir("/sys/class/hwmon") {
            for d in dirs.flatten() {
                let name = read(&format!("{}/name", d.path().display()));
                if matches!(name.trim(), "coretemp" | "k10temp" | "zenpower" | "cpu_thermal" | "acpitz") && d.path().join("temp1_input").exists() {
                    let better = name.trim() != "acpitz" || s.temp.is_none();
                    if better {
                        s.temp = Some(d.path().join("temp1_input"));
                    }
                }
                if s.fan.is_none() && d.path().join("fan1_input").exists() {
                    s.fan = Some(d.path().join("fan1_input"));
                }
            }
        }
        // AMD and Intel (xe) GPUs report their load in sysfs.
        if let Ok(cards) = std::fs::read_dir("/sys/class/drm") {
            for c in cards.flatten() {
                let busy = c.path().join("device/gpu_busy_percent");
                if busy.exists() {
                    s.gpu = Some(busy);
                    break;
                }
            }
        }
        s
    }

    fn cpu(&mut self) -> Option<f64> {
        let stat = read("/proc/stat");
        let f: Vec<u64> = stat.lines().next()?.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
        let idle = f.get(3)? + f.get(4).unwrap_or(&0);
        let total: u64 = f.iter().take(8).sum();
        let out = self.last_cpu.map(|(t, i)| {
            let (dt, di) = (total.saturating_sub(t), idle.saturating_sub(i));
            if dt == 0 { 0.0 } else { (dt - di) as f64 * 100.0 / dt as f64 }
        });
        self.last_cpu = Some((total, idle));
        out
    }

    fn memory(&self) -> Option<(f64, f64)> {
        let info = read("/proc/meminfo");
        let kb = |k: &str| info.lines().find_map(|l| l.strip_prefix(k)).and_then(|v| v.split_whitespace().next()?.parse::<f64>().ok());
        let (total, avail) = (kb("MemTotal:")?, kb("MemAvailable:")?);
        Some(((total - avail) * 100.0 / total, (total - avail) / 1024.0 / 1024.0))
    }

    fn temp(&self) -> Option<f64> {
        read(self.temp.as_ref()?.to_str()?).trim().parse::<f64>().ok().map(|m| m / 1000.0)
    }

    fn gpu(&self) -> Option<f64> {
        read(self.gpu.as_ref()?.to_str()?).trim().parse().ok()
    }

    /// The first fan's speed, while it turns.
    fn fan(&self) -> Option<u64> {
        read(self.fan.as_ref()?.to_str()?).trim().parse().ok().filter(|r| *r > 0)
    }
}

struct History {
    cpu: VecDeque<f64>,
    mem: VecDeque<f64>,
    temp: VecDeque<f64>,
    gpu: VecDeque<f64>,
}

/// The bar's sparkline shows the last this many samples.
const SPARK: usize = 16;

fn push(q: &mut VecDeque<f64>, v: f64) {
    // The first reading fills the sparkline, so it starts as a flat line, not a ramp from 0.
    if q.is_empty() {
        q.extend(std::iter::repeat_n(v, SPARK - 1));
    }
    if q.len() == HISTORY {
        q.pop_front();
    }
    q.push_back(v);
}

fn values(q: &VecDeque<f64>) -> Vec<f64> {
    q.iter().map(|v| (v * 10.0).round() / 10.0).collect()
}

/// Ten minutes at the default three seconds.
const DESKTOP_HISTORY: usize = 200;

fn desktop_view(w: &Widget, temp: bool, gpu: bool) -> Node {
    // Free sizes (it's resizable): three graphs when there's room, else two.
    let large = w.size == "large" || (w.size == "free" && w.height >= 280);
    let inner = (w.width - 32).max(64);
    let sections = if large && (temp || gpu) { 3 } else { 2 };
    let spacing = if large { 14 } else { 8 };
    let graph_h = match w.size.as_str() {
        "free" => (((w.height - 32 - (sections - 1) * spacing) / sections) - 22).clamp(20, 160),
        _ if large => 64,
        _ => 30,
    };
    let section = |name: &str, id: &str, tint: &str| {
        vbox([
            hbox([label(name).color("dim_text"), label("").expand(), label("–").id(format!("{id}-value"))]),
            graph(vec![], Some(100.0)).size(inner, graph_h).color(tint).id(format!("{id}-graph")),
        ])
        .spacing(4)
    };
    let mut rows = vec![section("CPU", "cpu", "accent"), section("Memory", "mem", "success")];
    if large {
        if temp {
            rows.push(section("Temperature", "temp", "warning"));
        } else if gpu {
            rows.push(section("GPU", "gpu", "highlight"));
        }
    }
    vbox(rows).spacing(spacing)
}

fn desktop(plugin: Plugin<()>, inputs: std::sync::mpsc::Receiver<Input<()>>) {
    let every = plugin.setting_i64("every", 3).clamp(1, 60) as u64;
    let tick = plugin.data();
    std::thread::spawn(move || {
        loop {
            tick(());
            std::thread::sleep(Duration::from_secs(every));
        }
    });
    let mut s = Sampler::new();
    let (has_temp, has_gpu) = (s.temp.is_some(), s.gpu.is_some());
    let mut q = [VecDeque::new(), VecDeque::new(), VecDeque::new(), VecDeque::new()];
    let mut now = [None::<f64>; 4];
    let mut widgets: Vec<Widget> = plugin.widgets().to_vec();
    let mut visible = true;
    let send_items = |widgets: &[Widget]| {
        for w in widgets.iter().filter(|w| w.item == "graphs") {
            plugin.item(item("graphs").monitor(&w.monitor).view(desktop_view(w, has_temp, has_gpu)));
        }
    };
    let send_values = |q: &[VecDeque<f64>; 4], now: &[Option<f64>; 4]| {
        let names = ["cpu", "mem", "temp", "gpu"];
        for (i, name) in names.iter().enumerate() {
            let Some(v) = now[i] else { continue };
            let text = if *name == "temp" { format!("{v:.0} °C") } else { format!("{v:.0}%") };
            plugin.patch("graphs", &format!("{name}-value"), [("text", text.into())]);
            plugin.patch("graphs", &format!("{name}-graph"), [("values", values(&q[i]).into())]);
        }
    };
    send_items(&widgets);
    for input in inputs {
        match input {
            Input::Data(()) => {
                let Some(cpu) = s.cpu() else { continue };
                let samples = [Some(cpu), s.memory().map(|m| m.0), s.temp(), s.gpu()];
                for (i, v) in samples.into_iter().enumerate() {
                    if let Some(v) = v {
                        if q[i].len() == DESKTOP_HISTORY {
                            q[i].pop_front();
                        }
                        q[i].push_back(v);
                    }
                    now[i] = v;
                }
                if visible {
                    send_values(&q, &now);
                }
            }
            Input::Visible(v) => {
                if v && !visible {
                    send_values(&q, &now);
                }
                visible = v;
            }
            Input::Widgets(w) => {
                widgets = w;
                send_items(&widgets);
                send_values(&q, &now);
            }
            Input::Shutdown => break,
            _ => {}
        }
    }
}

/// The busiest apps since the last look: (app, CPU %, resident bytes). Processes count
/// under the app whose program they run (a browser's many, an app's helpers), else under
/// their program's name.
#[derive(Default)]
struct Procs {
    last: std::collections::HashMap<u32, u64>,
    last_total: u64,
}

fn app_of(pid: u32, comm: &str) -> nyx_bar_apps::App {
    let program = std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .and_then(|p| p.file_name().map(|f| f.to_string_lossy().trim_end_matches(" (deleted)").to_string()))
        .unwrap_or_else(|| comm.to_string());
    nyx_bar_apps::for_program(&program).unwrap_or_else(|| nyx_bar_apps::lookup(&program))
}

impl Procs {
    fn top(&mut self, n: usize) -> Vec<(nyx_bar_apps::App, f64, u64)> {
        let total: u64 = read("/proc/stat").lines().next().map(|l| l.split_whitespace().skip(1).take(8).filter_map(|v| v.parse::<u64>().ok()).sum()).unwrap_or(0);
        let dt = total.saturating_sub(self.last_total).max(1) as f64;
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
        let page = 4096u64;
        let mut by_app: std::collections::HashMap<String, (nyx_bar_apps::App, f64, u64)> = Default::default();
        let mut seen = std::collections::HashMap::new();
        if let Ok(rd) = std::fs::read_dir("/proc") {
            for e in rd.flatten() {
                let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
                let stat = read(&format!("/proc/{pid}/stat"));
                let Some(close) = stat.rfind(')') else { continue };
                let name = stat[stat.find('(').map_or(0, |i| i + 1)..close].to_string();
                let f: Vec<&str> = stat[close + 1..].split_whitespace().collect();
                let ticks = f.get(11).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) + f.get(12).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
                let rss = f.get(21).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) * page;
                let cpu = self.last.get(&pid).map_or(0.0, |p| ticks.saturating_sub(*p) as f64 * 100.0 * cores / dt / cores);
                seen.insert(pid, ticks);
                if rss == 0 {
                    continue;
                }
                let app = app_of(pid, &name);
                let entry = by_app.entry(app.name.clone()).or_insert_with(|| (app, 0.0, 0));
                entry.1 += cpu;
                entry.2 += rss;
            }
        }
        let first = self.last.is_empty();
        self.last = seen;
        self.last_total = total;
        if first {
            return vec![];
        }
        let mut list: Vec<(nyx_bar_apps::App, f64, u64)> = by_app.into_values().collect();
        list.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(b.2.cmp(&a.2)));
        list.truncate(n);
        list
    }
}

/// Each core's load since the last sample.
#[derive(Default)]
struct Cores {
    last: Vec<(u64, u64)>,
}

impl Cores {
    fn sample(&mut self) -> Vec<f64> {
        let stat = read("/proc/stat");
        let now: Vec<(u64, u64)> = stat
            .lines()
            .filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(|b| b.is_ascii_digit()))
            .map(|l| {
                let f: Vec<u64> = l.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
                (f.iter().take(8).sum(), f.get(3).unwrap_or(&0) + f.get(4).unwrap_or(&0))
            })
            .collect();
        let out = now
            .iter()
            .zip(self.last.iter())
            .map(|((t, i), (lt, li))| {
                let (dt, di) = (t.saturating_sub(*lt), i.saturating_sub(*li));
                if dt == 0 { 0.0 } else { (dt - di) as f64 * 100.0 / dt as f64 }
            })
            .collect();
        self.last = now;
        out
    }
}

fn ghz() -> Option<f64> {
    let info = read("/proc/cpuinfo");
    let mhz: Vec<f64> = info.lines().filter_map(|l| l.strip_prefix("cpu MHz")).filter_map(|v| v.trim_start_matches([' ', '\t', ':']).trim().parse().ok()).collect();
    (!mhz.is_empty()).then(|| mhz.iter().sum::<f64>() / mhz.len() as f64 / 1000.0)
}

/// Free and total bytes on `/`.
fn disk() -> Option<(u64, u64)> {
    let path = std::ffi::CString::new("/").ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(path.as_ptr(), &mut st) } == 0).then(|| (st.f_bavail as u64 * st.f_frsize as u64, st.f_blocks as u64 * st.f_frsize as u64))
}

/// Received and sent bytes on every interface but loopback.
fn net_bytes() -> (u64, u64) {
    read("/proc/net/dev")
        .lines()
        .skip(2)
        .filter_map(|l| {
            let (name, rest) = l.split_once(':')?;
            if name.trim() == "lo" {
                return None;
            }
            let f: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            Some((*f.first()?, *f.get(8)?))
        })
        .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
}

fn rate(bytes_per_s: f64) -> String {
    match bytes_per_s {
        b if b >= 1e6 => format!("{:.1} MB/s", b / 1e6),
        b if b >= 1e3 => format!("{:.0} KB/s", b / 1e3),
        b => format!("{b:.0} B/s"),
    }
}

fn size(bytes: u64) -> String {
    let b = bytes as f64;
    match b {
        b if b >= 1e9 => format!("{:.1} GB", b / 1e9),
        b => format!("{:.0} MB", b / 1e6),
    }
}

/// CPU, memory and temperature glyphs on the 24 px grid (no icon theme has them all).
fn glyph(kind: &str, tint: &str) -> Node {
    let st = |d: &str| Draw::Path { d: d.into(), fill: None, stroke: Some(color(tint)), width: Some(1.7) };
    let cmds = match kind {
        "cpu" => vec![
            Draw::Rect { x: 6.0, y: 6.0, w: 12.0, h: 12.0, radius: 2.0, fill: None, stroke: Some(color(tint)), width: Some(1.7) },
            Draw::Rect { x: 9.5, y: 9.5, w: 5.0, h: 5.0, radius: 1.0, fill: None, stroke: Some(color(tint)), width: Some(1.7) },
            st("M9 3v3M15 3v3M9 18v3M15 18v3M3 9h3M3 15h3M18 9h3M18 15h3"),
        ],
        "memory" => vec![
            Draw::Rect { x: 3.0, y: 7.0, w: 18.0, h: 10.0, radius: 1.5, fill: None, stroke: Some(color(tint)), width: Some(1.7) },
            st("M7 10.5v3M11 10.5v3M15 10.5v3M6 17v2.5M10 17v2.5M14 17v2.5M18 17v2.5"),
        ],
        _ => vec![st("M10 4a2 2 0 0 1 4 0v10a4 4 0 1 1-4 0zM12 9v6")],
    };
    vector([0.0, 0.0, 24.0, 24.0], cmds).size(16, 16)
}

/// Each core's load as a row of pills (rows of eight on bigger processors), filled from
/// the bottom.
fn cores_view(loads: &[f64], width: i32, tint: &str) -> Node {
    let per_row = loads.len().clamp(1, 8);
    let rows = loads.len().div_ceil(per_row);
    let (gap, h) = (4.0, if rows > 1 { 12.0 } else { 18.0 });
    let w = (width as f64 - gap * (per_row - 1) as f64) / per_row as f64;
    let mut cmds = vec![];
    for (i, load) in loads.iter().enumerate() {
        let (x, y) = ((i % per_row) as f64 * (w + gap), (i / per_row) as f64 * (h + gap));
        let f = (load / 100.0).clamp(0.0, 1.0) * h;
        cmds.push(Draw::Rect { x, y, w, h, radius: 3.0, fill: Some(color("track")), stroke: None, width: None });
        if f >= 1.0 {
            cmds.push(Draw::Rect { x, y: y + h - f, w, h: f, radius: 3.0, fill: Some(color(tint)), stroke: None, width: None });
        }
    }
    let total = rows as f64 * h + (rows - 1) as f64 * gap;
    vector([0.0, 0.0, width as f64, total], cmds).size(width, total as i32).tooltip(&format!("{} cores", loads.len()))
}

/// 2100 → "2,100".
fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The GPU's temperature, from its hwmon sensor (amdgpu, i915, xe, nouveau).
fn temp_gpu() -> Option<f64> {
    for d in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let name = read(&format!("{}/name", d.path().display()));
        if matches!(name.trim(), "amdgpu" | "i915" | "xe" | "nouveau") {
            return read(&format!("{}/temp1_input", d.path().display())).trim().parse::<f64>().ok().map(|m| m / 1000.0);
        }
    }
    None
}

fn filled(mut n: Node) -> Node {
    if let NodeKind::Box { align, .. } = &mut n.kind {
        *align = Align::Fill;
    }
    n
}

/// A short label beside an expanding one keeps its words (popover labels wrap).
fn keep(text: impl Into<String>, px_per_char: i32) -> Node {
    let t: String = text.into();
    let w = t.chars().count() as i32 * px_per_char + 8;
    label(t).width(w)
}

/// A small ring: `pct` of it in `tint`.
fn ring(pct: f64, tint: &str) -> Node {
    let end = -90.0 + 360.0 * (pct / 100.0).clamp(0.0, 1.0);
    vector(
        [0.0, 0.0, 16.0, 16.0],
        vec![
            Draw::Circle { cx: 8.0, cy: 8.0, r: 6.5, fill: None, stroke: Some(color("track")), width: Some(2.4) },
            Draw::Arc { cx: 8.0, cy: 8.0, r: 6.5, start: -90.0, end, fill: None, stroke: Some(color(tint)), width: Some(2.4), round: true },
        ],
    )
    .size(16, 16)
}

#[derive(Clone, Copy, PartialEq)]
enum Look {
    Spark,
    Text,
    Rings,
}

fn look_of(v: &serde_json::Value) -> Look {
    match v.get("look").and_then(|l| l.as_str()) {
        Some("text") => Look::Text,
        Some("rings") => Look::Rings,
        _ => Look::Spark,
    }
}

fn card(children: Vec<Node>) -> Node {
    vbox(children).spacing(6).class("card")
}

fn main() {
    let (plugin, inputs) = Plugin::<()>::start();
    if plugin.hello.kind == "desktop" {
        return desktop(plugin, inputs);
    }
    let mut every = plugin.setting_i64("every", 3).clamp(1, 60) as u64;
    let mut look = look_of(plugin.settings());
    let mut show_gpu = plugin.setting_bool("gpu", true);
    let mut show_memory = plugin.setting_bool("memory", true);
    let tick = plugin.data();
    let interval = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(every));
    let i2 = interval.clone();
    std::thread::spawn(move || {
        loop {
            tick(());
            std::thread::sleep(Duration::from_secs(i2.load(std::sync::atomic::Ordering::Relaxed)));
        }
    });
    let mut s = Sampler::new();
    let mut h = History { cpu: VecDeque::new(), mem: VecDeque::new(), temp: VecDeque::new(), gpu: VecDeque::new() };
    let mut net_hist: VecDeque<f64> = VecDeque::new();
    let mut cores = Cores::default();
    let mut procs = Procs::default();
    let mut last_net: Option<(u64, u64, std::time::Instant)> = None;
    let mut visible = true;
    let mut popover_open = false;
    let activity = have("nyx-activity");
    for input in inputs {
        match input {
            Input::Data(()) if visible => {
                let Some(cpu) = s.cpu() else { continue };
                let (mem, used_gb) = s.memory().unwrap_or((0.0, 0.0));
                let temp = s.temp();
                let gpu = if show_gpu { s.gpu() } else { None };
                push(&mut h.cpu, cpu);
                push(&mut h.mem, mem);
                if let Some(t) = temp {
                    push(&mut h.temp, t);
                }
                if let Some(g) = gpu {
                    push(&mut h.gpu, g);
                }
                let (rx, tx) = net_bytes();
                let now = std::time::Instant::now();
                let (down, up) = match last_net {
                    Some((r, t, at)) => {
                        let secs = now.duration_since(at).as_secs_f64().max(0.1);
                        (rx.saturating_sub(r) as f64 / secs, tx.saturating_sub(t) as f64 / secs)
                    }
                    None => (0.0, 0.0),
                };
                last_net = Some((rx, tx, now));
                push(&mut net_hist, down);
                let per_core = cores.sample();
                let busy = cpu >= 85.0;
                let hot = temp.is_some_and(|t| t >= 85.0);
                let tint = if busy { "warning" } else { "accent" };
                let mut parts = vec![];
                match look {
                    Look::Spark => {
                        parts.push(glyph("cpu", if busy { "warning" } else { "dim_text" }));
                        let recent: Vec<f64> = values(&h.cpu).into_iter().rev().take(SPARK).rev().collect();
                        // Twice the recent peak (at least 25 %), so a quiet line still has a shape.
                        let top = (recent.iter().cloned().fold(0.0, f64::max) * 2.0).clamp(25.0, 100.0);
                        parts.push(graph(recent, Some(top)).size(34, 14).color(tint));
                        let mut l = label(format!("{cpu:.0}%"));
                        if busy {
                            l = l.class("warning");
                        }
                        parts.push(l);
                        if show_memory {
                            let full = mem >= 90.0;
                            parts.push(glyph("memory", if full { "warning" } else { "dim_text" }).margin(0, 0, 0, 4));
                            parts.push(label(format!("{mem:.0}%")).class(if full { "warning" } else { "plain" }));
                        }
                    }
                    Look::Text => {
                        parts.push(hbox([glyph("cpu", "dim_text"), label(format!("{cpu:.0}%")).class(if busy { "warning" } else { "plain" })]).spacing(4));
                        parts.push(hbox([glyph("memory", "dim_text"), label(format!("{mem:.0}%"))]).spacing(4));
                        if let Some(t) = temp {
                            parts.push(hbox([glyph("temp", "dim_text"), label(format!("{t:.0}°")).class(if hot { "danger" } else { "plain" })]).spacing(4));
                        }
                    }
                    Look::Rings => {
                        parts.push(hbox([ring(cpu, tint), label("CPU").class("dim").class("xs")]).spacing(4));
                        parts.push(hbox([ring(mem, "highlight"), label("RAM").class("dim").class("xs")]).spacing(4));
                        if let Some(t) = temp {
                            parts.push(hbox([ring(t, if hot { "danger" } else { "warning" }), label(format!("{t:.0}°")).class("dim").class("xs")]).spacing(4));
                        }
                    }
                }
                let mut tip = format!("CPU {cpu:.0}% · memory {mem:.0}% ({used_gb:.1} GB)");
                if let Some(t) = temp {
                    tip.push_str(&format!(" · {t:.0} °C"));
                }
                if let Some(g) = gpu {
                    tip.push_str(&format!(" · GPU {g:.0}%"));
                }
                let mut it = item("stats").view(hbox(parts).spacing(if look == Look::Spark { 6 } else { 10 })).tooltip(tip);
                if activity {
                    it = it.on_right_click(&["nyx-activity", "--toggle"]);
                }
                plugin.item(it);
                if popover_open || h.cpu.len() <= SPARK {
                    let top = procs.top(4);
                    let total_gb = read("/proc/meminfo").lines().find_map(|l| l.strip_prefix("MemTotal:")).and_then(|v| v.split_whitespace().next()?.parse::<f64>().ok()).map_or(0.0, |kb| kb / 1024.0 / 1024.0);
                    let mut caption = format!("{} cores", per_core.len().max(1));
                    if let Some(g) = ghz() {
                        caption.push_str(&format!(" · {g:.1} GHz"));
                    }
                    if let Some(t) = temp {
                        caption.push_str(&format!(" · {t:.0} °C"));
                    }
                    if let Some(f) = s.fan() {
                        caption.push_str(&format!(" · fan {} rpm", thousands(f)));
                    }
                    let mut cpu_card = vec![
                        hbox([glyph("cpu", "dim_text"), label("CPU").class("strong"), label("").expand(), label(format!("{cpu:.0}%")).class("sbig").class(if busy { "warning" } else { "plain" })]).spacing(8),
                        graph(values(&h.cpu), Some(100.0)).size(332, 44).color(tint),
                    ];
                    if !per_core.is_empty() {
                        cpu_card.push(cores_view(&per_core, 332, tint));
                    }
                    cpu_card.push(label(caption).class("tertiary").class("xs"));
                    let small = |pic: Node, title: &str, value: String, below: Node| {
                        let head = hbox([label(title).class("strong"), label("").expand(), label(value)]).spacing(6).class("srow");
                        hbox([pic, vbox([head, below]).spacing(7).expand()]).spacing(10).class("scard").width(173)
                    };
                    let rail = |f: f64| progress(f.clamp(0.0, 1.0)).class("rail");
                    let mut grid = vec![small(glyph("memory", "dim_text"), "Memory", format!("{used_gb:.1} / {total_gb:.0} GB"), rail(mem / 100.0))];
                    if let Some(g) = gpu {
                        let v = match temp_gpu() {
                            Some(t) => format!("{g:.0}% · {t:.0}°"),
                            None => format!("{g:.0}%"),
                        };
                        grid.push(small(icon("video-display-symbolic").class("dim"), "GPU", v, rail(g / 100.0)));
                    } else if let Some(t) = temp {
                        grid.push(small(glyph("temp", if hot { "danger" } else { "dim_text" }), "Heat", format!("{t:.0} °C"), rail(t / 100.0)));
                    }
                    if let Some((free, total)) = disk() {
                        grid.push(small(icon("drive-harddisk-symbolic").class("dim"), "Disk", format!("{} free", size(free)), rail(1.0 - free as f64 / total.max(1) as f64)));
                    }
                    let spark = graph(net_hist.iter().copied().collect(), None).size(120, 12).color("accent").tooltip(&format!("Down {} · up {}", rate(down), rate(up)));
                    grid.push(small(icon("network-transmit-receive-symbolic|network-wired-symbolic").class("dim"), "Network", rate(down), spark));
                    let mut rows = vec![card(cpu_card).class("scard").class("cpu")];
                    let cells: Vec<Node> = grid.chunks(2).map(|pair| filled(hbox(pair.to_vec()).spacing(6))).collect();
                    rows.push(vbox(cells).spacing(6));
                    if !top.is_empty() {
                        let mut list = vec![hbox([label("TOP PROCESSES").class("caps").width(120), label("").expand(), keep("CPU", 8).class("caps")]).margin(6, 6, 2, 6)];
                        for (app, c, m) in top {
                            list.push(hbox([icon_sized(&app.icons(), 20), label(&app.name).expand(), keep(size(m), 7).class("tertiary").class("small"), label(format!("{c:.1}%")).class("pc").width(44)]).spacing(10).class("proc"));
                        }
                        rows.push(vbox(list).spacing(0));
                    }
                    if activity {
                        rows.push(footer("activity", "Open Activity", Some(label(format!("Updates every {every} s")).class("tertiary").class("xs"))));
                    }
                    plugin.popover("stats", Some(vbox(rows).spacing(6).width(356)));
                }
            }
            Input::Visible(v) => visible = v,
            Input::Settings(st) => {
                every = st.get("every").and_then(|v| v.as_i64()).unwrap_or(3).clamp(1, 60) as u64;
                interval.store(every, std::sync::atomic::Ordering::Relaxed);
                look = look_of(&st);
                show_gpu = st.get("gpu").and_then(|v| v.as_bool()).unwrap_or(true);
                show_memory = st.get("memory").and_then(|v| v.as_bool()).unwrap_or(true);
            }
            Input::Event(e) => match e.kind {
                nyx_plugin_sdk::protocol::EventKind::PopoverOpen => popover_open = true,
                nyx_plugin_sdk::protocol::EventKind::PopoverClose => popover_open = false,
                nyx_plugin_sdk::protocol::EventKind::Action if e.control.as_deref() == Some("activity") => {
                    nyx_plugin_sdk::spawn(&["nyx-activity", "--toggle"]);
                }
                _ => {}
            },
            Input::Shutdown => break,
            _ => {}
        }
    }
}
