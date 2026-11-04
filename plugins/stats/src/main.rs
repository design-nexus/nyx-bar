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
}

struct History {
    cpu: VecDeque<f64>,
    mem: VecDeque<f64>,
    temp: VecDeque<f64>,
    gpu: VecDeque<f64>,
}

fn push(q: &mut VecDeque<f64>, v: f64) {
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

fn main() {
    let (plugin, inputs) = Plugin::<()>::start();
    if plugin.hello.kind == "desktop" {
        return desktop(plugin, inputs);
    }
    let mut every = plugin.setting_i64("every", 3).clamp(1, 60) as u64;
    let mut graphs = plugin.setting_bool("graphs", false);
    let mut show_gpu = plugin.setting_bool("gpu", true);
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
                let hot = temp.is_some_and(|t| t >= 85.0);
                let mut parts = vec![];
                if graphs {
                    parts.push(icon("cpu-symbolic").tooltip("CPU"));
                    parts.push(graph(values(&h.cpu), Some(100.0)).size(36, 18));
                    parts.push(icon("memory-symbolic"));
                    parts.push(graph(values(&h.mem), Some(100.0)).size(36, 18).color("success"));
                } else {
                    parts.push(markup(format!("<span alpha=\"60%\">CPU</span> {cpu:.0}%")));
                    parts.push(markup(format!("<span alpha=\"60%\">RAM</span> {mem:.0}%")));
                    if let Some(g) = gpu {
                        parts.push(markup(format!("<span alpha=\"60%\">GPU</span> {g:.0}%")));
                    }
                }
                if let Some(t) = temp {
                    let mut l = label(format!("{t:.0}°"));
                    if hot {
                        l = l.color("danger");
                    }
                    parts.push(l);
                }
                let mut tip = format!("CPU {cpu:.0}% · memory {mem:.0}% ({used_gb:.1} GB)");
                if let Some(t) = temp {
                    tip.push_str(&format!(" · {t:.0} °C"));
                }
                if let Some(g) = gpu {
                    tip.push_str(&format!(" · GPU {g:.0}%"));
                }
                let mut it = item("stats").view(hbox(parts).spacing(8)).tooltip(tip);
                if activity {
                    it = it.on_right_click(&["nyx-activity", "--toggle"]);
                }
                plugin.item(it);
                if popover_open || h.cpu.len() <= 1 {
                    let section = |title: String, q: &VecDeque<f64>, max: Option<f64>, tint: &str| {
                        vbox([label(title).color("dim_text"), graph(values(q), max).size(300, 48).color(tint)]).spacing(2)
                    };
                    let mut rows = vec![
                        header("System"),
                        section(format!("CPU · {cpu:.0}%"), &h.cpu, Some(100.0), "accent"),
                        section(format!("Memory · {mem:.0}% · {used_gb:.1} GB"), &h.mem, Some(100.0), "success"),
                    ];
                    if let Some(t) = temp {
                        rows.push(section(format!("Temperature · {t:.0} °C"), &h.temp, Some(100.0), if hot { "danger" } else { "warning" }));
                    }
                    if let Some(g) = gpu {
                        rows.push(section(format!("GPU · {g:.0}%"), &h.gpu, Some(100.0), "highlight"));
                    }
                    if activity {
                        rows.push(button("activity", Some("Open Activity"), Some("go-next-symbolic")));
                    }
                    plugin.popover("stats", Some(vbox(rows).spacing(10)));
                }
            }
            Input::Visible(v) => visible = v,
            Input::Settings(st) => {
                every = st.get("every").and_then(|v| v.as_i64()).unwrap_or(3).clamp(1, 60) as u64;
                interval.store(every, std::sync::atomic::Ordering::Relaxed);
                graphs = st.get("graphs").and_then(|v| v.as_bool()).unwrap_or(false);
                show_gpu = st.get("gpu").and_then(|v| v.as_bool()).unwrap_or(true);
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
