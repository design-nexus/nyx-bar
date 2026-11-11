//! The bar's plugins and layout without GTK, as the GTK bar's `bar.rs` keeps them: found
//! in the three plugin folders, run while one of their bar parts is turned on (and, for
//! third-party plugins, approved), restarted when their folder, manifest, parts or limits
//! change. What they show goes to the interface as topic `bar`; their patches go on as
//! `Patch` events.

use gio::prelude::*;
use nyx_bar_core::{Config, Shown, Slot, arrange, should_hide};
use nyx_desktop::{Desktop, watch};
use nyx_plugin::{Kind, Plugin, State, discovery, paths};
use nyx_plugin_host::{self as host, Handle, Report, Spec, limits::Limits};
use nyx_plugin_protocol::{
    API_VERSION, Align, Anchor, DesktopMonitor, DesktopState, DesktopWorkspace, Direction, Event, EventKind, Hello, HostMsg, Item,
    MenuEntry, MonitorInfo, Node, NodeKind, Orientation, PluginMsg, ThemeInfo,
};
use serde_json::{Map, Value, json};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

type Key = (String, Option<String>);

#[derive(Debug, Clone, PartialEq)]
pub enum RunState {
    Starting,
    Running { pid: u32, scoped: bool, sandboxed: bool },
    Crashed(String),
    Unavailable(String),
}

pub struct Running {
    pub plugin: Plugin,
    pub handle: Handle,
    pub parts: Vec<String>,
    settings: Value,
    limits: Limits,
    /// Items in the order they first came.
    pub items: Vec<Item>,
    /// Each item's revision: up when the plugin sends it again, not on a patch.
    revs: HashMap<Key, u64>,
    menus: HashMap<Key, Vec<MenuEntry>>,
    popovers: HashMap<Key, Node>,
    pub state: RunState,
    generation: u64,
    warned: HashSet<String>,
}

pub struct Shared {
    pub link: nyx_ui_link::Link,
    pub only: Option<String>,
    config: RefCell<Config>,
    desktop: RefCell<Arc<Desktop>>,
    state: RefCell<State>,
    pub plugins: RefCell<BTreeMap<String, Running>>,
    /// Each plugin's style.css, compiled for QML.
    styles: RefCell<HashMap<String, Value>>,
    hidden: RefCell<HashMap<String, bool>>,
    visible: Cell<bool>,
    rev: Cell<u64>,
    generation: Cell<u64>,
    sync_pending: Cell<bool>,
    publish_pending: Cell<bool>,
    monitors: RefCell<Vec<gio::FileMonitor>>,
    plugin_monitors: RefCell<HashMap<String, gio::FileMonitor>>,
    changed_plugins: RefCell<HashSet<String>>,
}

pub fn debug(f: impl FnOnce() -> String) {
    if std::env::var_os("NYX_BAR_DEBUG").is_some() {
        eprintln!("nyx-bar-backend: {}", f());
    }
}

impl Shared {
    pub fn test(&self) -> bool {
        self.only.is_some()
    }

    fn host_name(&self) -> &'static str {
        if self.test() { "bar-test" } else { "bar" }
    }
}

pub fn start(link: nyx_ui_link::Link, only: Option<String>) -> Rc<Shared> {
    nyx_bar_core::prepare_user_file();
    let config = Config::load_layered(&nyx_config::Layers::standard()).unwrap_or_else(|e| {
        eprintln!("nyx-bar-backend: {e}; using the default layout");
        Config::default()
    });
    let s = Rc::new(Shared {
        link,
        only,
        config: RefCell::new(config),
        desktop: RefCell::new(Arc::new(Desktop::default())),
        state: RefCell::new(State::load()),
        plugins: RefCell::default(),
        styles: RefCell::default(),
        hidden: RefCell::default(),
        visible: Cell::new(true),
        rev: Cell::new(0),
        generation: Cell::new(0),
        sync_pending: Cell::new(false),
        publish_pending: Cell::new(false),
        monitors: RefCell::default(),
        plugin_monitors: RefCell::default(),
        changed_plugins: RefCell::default(),
    });
    host::reap_stale(s.host_name());

    let ws = Rc::downgrade(&s);
    std::mem::forget(watch::watch_local(move |u| {
        let Some(s) = ws.upgrade() else { return };
        *s.desktop.borrow_mut() = u.desktop.clone();
        if u.changes.monitors {
            let monitors = monitor_infos(&s);
            broadcast(&s, |_| Some(HostMsg::Monitors { monitors: monitors.clone() }));
            publish(&s);
        }
        if u.changes.windows || u.changes.workspaces || u.changes.active || u.changes.monitors {
            update_hiding(&s);
        }
        if u.changes.workspaces || u.changes.active || u.changes.focus || u.changes.titles || u.changes.windows {
            let state = desktop_state(&u.desktop);
            broadcast(&s, |r| r.plugin.manifest.permissions.desktop.then(|| HostMsg::Desktop(state.clone())));
        }
    }));
    watch_files(&s);
    follow_theme(&s);
    sync_plugins(&s);
    s
}

// ---------- Files and theme ----------

fn monitor_file(path: &Path, f: impl Fn() + 'static) -> Option<gio::FileMonitor> {
    let m = gio::File::for_path(path).monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE).ok()?;
    m.connect_changed(move |_, _, _, ev| {
        if matches!(
            ev,
            gio::FileMonitorEvent::ChangesDoneHint
                | gio::FileMonitorEvent::Created
                | gio::FileMonitorEvent::Deleted
                | gio::FileMonitorEvent::MovedIn
                | gio::FileMonitorEvent::Renamed
        ) {
            f();
        }
    });
    Some(m)
}

fn monitor_dir(path: &Path, f: impl Fn(Option<String>) + 'static) -> Option<gio::FileMonitor> {
    let _ = std::fs::create_dir_all(path);
    let m = gio::File::for_path(path).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE).ok()?;
    m.connect_changed(move |_, file, _, _| f(file.basename().map(|b| b.to_string_lossy().into_owned())));
    Some(m)
}

fn watch_files(s: &Rc<Shared>) {
    let mut monitors = vec![];
    // The layout in any layer (by hand, Settings, or NyxOS's defaults in an update).
    let (tx, rx) = async_channel::unbounded::<()>();
    let store = nyx_config::Store::new(nyx_bar_core::DOMAIN, toml::Table::new());
    match store.watch(move || {
        let _ = tx.send_blocking(());
    }) {
        Ok(w) => std::mem::forget(w),
        Err(e) => eprintln!("nyx-bar-backend: can't watch the layout: {e:#}"),
    }
    let ws = Rc::downgrade(s);
    glib::spawn_future_local(async move {
        while rx.recv().await.is_ok() {
            glib::timeout_future(Duration::from_millis(150)).await;
            while rx.try_recv().is_ok() {}
            let Some(s) = ws.upgrade() else { return };
            match Config::load_layered(&nyx_config::Layers::standard()) {
                Ok(c) => {
                    *s.config.borrow_mut() = c;
                    schedule_sync(&s);
                    update_hiding(&s);
                    publish(&s);
                }
                Err(e) => eprintln!("nyx-bar-backend: {e}; keeping the layout as it was"),
            }
        }
    });
    let ws = Rc::downgrade(s);
    monitors.extend(monitor_file(&paths::state_file(), move || {
        if let Some(s) = ws.upgrade() {
            schedule_sync(&s);
        }
    }));
    for (dir, _) in discovery::search_dirs() {
        let ws = Rc::downgrade(s);
        monitors.extend(monitor_dir(&dir, move |name| {
            let Some(s) = ws.upgrade() else { return };
            if !name.as_deref().is_some_and(|n| n.starts_with('.')) {
                schedule_sync(&s);
            }
        }));
    }
    *s.monitors.borrow_mut() = monitors;
}

/// Keep the theme snapshot current, and tell plugins of a change.
fn follow_theme(s: &Rc<Shared>) {
    if !s.test() {
        nyx_theme_core::notify_changed("system", "shell");
    }
    let pending = Rc::new(Cell::new(false));
    let mut monitors = Vec::new();
    for dir in nyx_theme_core::paths::watched_dirs() {
        let Ok(m) = gio::File::for_path(&dir).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) else {
            continue;
        };
        let (pending, ws) = (pending.clone(), Rc::downgrade(s));
        m.connect_changed(move |_, file, _, _| {
            if file.basename().is_some_and(|b| b.to_string_lossy().ends_with(".tmp")) || pending.replace(true) {
                return;
            }
            let (pending, ws) = (pending.clone(), ws.clone());
            glib::timeout_add_local_once(Duration::from_millis(120), move || {
                pending.set(false);
                let Some(s) = ws.upgrade() else { return };
                let before = nyx_theme_core::snapshot::generation();
                let now = if s.test() { before } else { nyx_theme_core::notify_changed("system", "shell") };
                if now != before || s.test() {
                    let theme = theme_info();
                    broadcast(&s, |_| Some(HostMsg::Theme { theme: theme.clone() }));
                }
            });
        });
        monitors.push(m);
    }
    std::mem::forget(monitors);
}

pub fn theme_info() -> ThemeInfo {
    let snap = nyx_theme_core::snapshot::read();
    ThemeInfo {
        light: snap.as_ref().is_some_and(|s| s.legacy.get("light").is_some_and(|v| v == "true")),
        name: snap.as_ref().map(|s| s.theme.id.clone()).unwrap_or_default(),
        generation: nyx_theme_core::snapshot::generation(),
    }
}

/// Watch each running plugin's folder: an edit restarts it.
fn watch_plugin_dirs(s: &Rc<Shared>) {
    let running: Vec<(String, PathBuf)> = s.plugins.borrow().iter().map(|(id, r)| (id.clone(), r.plugin.dir.clone())).collect();
    let mut mons = s.plugin_monitors.borrow_mut();
    mons.retain(|id, _| running.iter().any(|(r, _)| r == id));
    for (id, dir) in running {
        if mons.contains_key(&id) {
            continue;
        }
        let ws = Rc::downgrade(s);
        let pid = id.clone();
        if let Some(m) = monitor_dir(&dir.canonicalize().unwrap_or(dir), move |name| {
            let Some(s) = ws.upgrade() else { return };
            if name.as_deref() == Some(discovery::ORIGIN_FILE) {
                return;
            }
            s.changed_plugins.borrow_mut().insert(pid.clone());
            schedule_sync(&s);
        }) {
            mons.insert(id, m);
        }
    }
}

pub fn schedule_sync(s: &Rc<Shared>) {
    if s.sync_pending.replace(true) {
        return;
    }
    let ws = Rc::downgrade(s);
    glib::timeout_add_local_once(Duration::from_millis(300), move || {
        if let Some(s) = ws.upgrade() {
            s.sync_pending.set(false);
            sync_plugins(&s);
        }
    });
}

/// The interface's state, once however many changes come together.
pub fn publish(s: &Rc<Shared>) {
    if s.publish_pending.replace(true) {
        return;
    }
    let ws = Rc::downgrade(s);
    glib::idle_add_local_once(move || {
        if let Some(s) = ws.upgrade() {
            s.publish_pending.set(false);
            s.link.publish("bar", state_json(&s));
        }
    });
}

// ---------- Plugins ----------

fn monitor_infos(s: &Shared) -> Vec<MonitorInfo> {
    s.desktop
        .borrow()
        .monitors
        .iter()
        .filter(|m| !m.disabled)
        .map(|m| {
            let (w, h) = m.logical_size();
            MonitorInfo { name: m.name.clone(), stable_id: m.stable_id(), width: w, height: h, scale: m.scale, focused: m.focused }
        })
        .collect()
}

fn desktop_state(d: &Desktop) -> DesktopState {
    let active = d.active_window.as_deref().and_then(|a| d.window(a));
    DesktopState {
        monitors: d.monitors.iter().map(|m| DesktopMonitor { name: m.name.clone(), active_workspace: m.active_workspace, focused: m.focused }).collect(),
        workspaces: d
            .workspaces
            .iter()
            .filter(|w| d.workspace_number(w).is_some())
            .map(|w| DesktopWorkspace {
                id: w.id,
                name: w.name.clone(),
                monitor: w.monitor.clone(),
                windows: w.windows,
                urgent: d.windows.iter().any(|x| x.workspace == w.id && x.urgent),
                fullscreen: w.has_fullscreen,
            })
            .collect(),
        active_class: active.map(|w| w.class.clone()),
        active_title: active.map(|w| w.title.clone()),
    }
}

fn broadcast(s: &Shared, msg: impl Fn(&Running) -> Option<HostMsg>) {
    for r in s.plugins.borrow().values() {
        if let Some(m) = msg(r) {
            r.handle.send(m);
        }
    }
}

/// Start, restart and stop plugins to match the folders, `plugins.toml` and `bar.toml`.
pub fn sync_plugins(s: &Rc<Shared>) {
    let found = discovery::discover();
    let state = State::load();
    let changed: HashSet<String> = s.changed_plugins.take();
    let wanted: Vec<Plugin> = found.plugins.into_iter().filter(|p| p.manifest.has_kind(Kind::Bar) && state.runs(p, Kind::Bar)).collect();
    *s.state.borrow_mut() = state.clone();
    let config = s.config.borrow().clone();
    let stale: Vec<String> = s.plugins.borrow().keys().filter(|id| !wanted.iter().any(|p| p.manifest.id == **id)).cloned().collect();
    for id in stale {
        debug(|| format!("stopping {id}"));
        s.plugins.borrow_mut().remove(&id);
        s.styles.borrow_mut().remove(&id);
    }
    for p in wanted {
        let id = p.manifest.id.clone();
        let parts = state.enabled_parts(&p.manifest, Kind::Bar);
        let settings = config.settings(&id, &p.manifest.settings);
        let limits = Limits::resolve(p.trust.is_trusted(), &p.manifest.limits, config.limits(&id));
        let keep = {
            let mut plugins = s.plugins.borrow_mut();
            match plugins.get_mut(&id) {
                Some(r)
                    if r.plugin.manifest == p.manifest
                        && r.plugin.dir == p.dir
                        && r.plugin.trust == p.trust
                        && r.parts == parts
                        && r.limits == limits
                        && !changed.contains(&id)
                        && !matches!(r.state, RunState::Unavailable(_)) =>
                {
                    if r.settings != settings {
                        r.handle.send(HostMsg::Settings { settings: settings.clone() });
                        r.settings = settings.clone();
                    }
                    true
                }
                _ => false,
            }
        };
        if !keep {
            debug(|| format!("starting {id} ({})", p.trust.label()));
            spawn_plugin(s, p, parts, settings, limits);
        }
    }
    watch_plugin_dirs(s);
    publish(s);
}

fn load_style(s: &Shared, p: &Plugin) {
    match std::fs::read_to_string(p.dir.join("style.css")) {
        Ok(text) => s.styles.borrow_mut().insert(p.manifest.id.clone(), crate::css::compile(&text)),
        Err(_) => s.styles.borrow_mut().remove(&p.manifest.id),
    };
}

fn spawn_plugin(s: &Rc<Shared>, p: Plugin, parts: Vec<String>, settings: Value, limits: Limits) {
    let id = p.manifest.id.clone();
    load_style(s, &p);
    let hello = Hello {
        api_version: API_VERSION,
        kind: Kind::Bar.name().into(),
        plugin: id.clone(),
        items: parts.clone(),
        settings: settings.clone(),
        monitors: monitor_infos(s),
        theme: theme_info(),
        permissions: vec![],
        frame_socket: None,
        bar_height: Some(s.config.borrow().height()),
        widgets: vec![],
    };
    let spec = Spec { plugin: p.clone(), kind: Kind::Bar, host: s.host_name().into(), hello, limits: limits.clone() };
    let (tx, rx) = async_channel::unbounded();
    let handle = host::spawn(spec, tx);
    let generation = s.generation.get() + 1;
    s.generation.set(generation);
    if !s.visible.get() {
        handle.send(HostMsg::Visible { visible: false, item: None });
    }
    if p.manifest.permissions.desktop && s.desktop.borrow().connected {
        handle.send(HostMsg::Desktop(desktop_state(&s.desktop.borrow())));
    }
    s.plugins.borrow_mut().insert(
        id.clone(),
        Running {
            plugin: p,
            handle,
            parts,
            settings,
            limits,
            items: vec![],
            revs: HashMap::new(),
            menus: HashMap::new(),
            popovers: HashMap::new(),
            state: RunState::Starting,
            generation,
            warned: HashSet::new(),
        },
    );
    let ws = Rc::downgrade(s);
    glib::spawn_future_local(async move {
        while let Ok(report) = rx.recv().await {
            let Some(s) = ws.upgrade() else { return };
            let current = s.plugins.borrow().get(&id).is_some_and(|r| r.generation == generation);
            if !current {
                return;
            }
            on_report(&s, &id, report);
        }
    });
}

fn warn_once(s: &Shared, id: &str, what: String) {
    let mut plugins = s.plugins.borrow_mut();
    if let Some(r) = plugins.get_mut(id)
        && r.warned.insert(what.clone())
    {
        eprintln!("nyx-bar-backend: {id}: {what}");
    }
}

fn on_report(s: &Rc<Shared>, id: &str, report: Report) {
    match report {
        Report::Started { pid, scoped, sandboxed } => {
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.state = RunState::Running { pid, scoped, sandboxed };
            }
        }
        Report::Message(m) => on_message(s, id, m),
        Report::Frames(_) => {}
        Report::Exited { status, stderr } => {
            eprintln!("nyx-bar-backend: {id} stopped ({status}){}", if stderr.is_empty() { String::new() } else { format!(": {stderr}") });
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.state = RunState::Crashed(status);
                r.items.clear();
            }
            publish(s);
        }
        Report::Suspended { reason, crashed } => {
            let name = s.plugins.borrow().get(id).map(|r| r.plugin.manifest.name.clone()).unwrap_or_else(|| id.to_string());
            eprintln!("nyx-bar-backend: {id} is off: {reason}");
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.state = RunState::Unavailable(reason.clone());
                r.items.clear();
            }
            publish(s);
            if crashed {
                if let Ok(mut e) = nyx_plugin::state::Editor::open() {
                    e.suspend(id, &reason, &nyx_plugin::state::now_utc());
                    let _ = e.save();
                }
                if std::env::var_os("NYX_BAR_NO_NOTIFY").is_some() {
                    return;
                }
                let _ = std::process::Command::new("notify-send")
                    .args([
                        "--app-name=Nyx Bar",
                        "--icon=dialog-warning",
                        &format!("{name} was turned off"),
                        &format!("Nyx Bar stopped it because {reason}. Turn it back on with: nyx plugin enable {id}"),
                    ])
                    .stdin(std::process::Stdio::null())
                    .spawn()
                    .map(|mut c| std::thread::spawn(move || c.wait()));
            }
        }
        Report::Warning(w) => warn_once(s, id, w),
    }
}

fn on_message(s: &Rc<Shared>, id: &str, msg: PluginMsg) {
    match msg {
        PluginMsg::Item(item) => {
            {
                let mut plugins = s.plugins.borrow_mut();
                let Some(r) = plugins.get_mut(id) else { return };
                let part = r.plugin.manifest.part_of(Kind::Bar, &item.id);
                if !part.as_ref().is_some_and(|p| r.parts.contains(p)) {
                    let what = format!("item {} isn't one of the parts turned on ({})", item.id, r.parts.join(", "));
                    drop(plugins);
                    warn_once(s, id, what);
                    return;
                }
                let rev = s.rev.get() + 1;
                s.rev.set(rev);
                r.revs.insert((item.id.clone(), item.monitor.clone()), rev);
                match r.items.iter_mut().find(|i| i.id == item.id && i.monitor == item.monitor) {
                    Some(old) => *old = item,
                    None => r.items.push(item),
                }
            }
            publish(s);
        }
        PluginMsg::Patch(p) => {
            {
                let mut plugins = s.plugins.borrow_mut();
                let Some(r) = plugins.get_mut(id) else { return };
                for item in r.items.iter_mut().filter(|i| i.id == p.item && (p.monitor.is_none() || i.monitor == p.monitor)) {
                    let mut view = item.view.take().unwrap_or_else(|| simple(item));
                    for n in &p.nodes {
                        view.apply_patch(&n.id, &n.set);
                    }
                    item.view = Some(view);
                }
                for (k, node) in r.popovers.iter_mut() {
                    if k.0 == p.item {
                        for n in &p.nodes {
                            node.apply_patch(&n.id, &n.set);
                        }
                    }
                }
            }
            // Patches are small and come often: the interface applies them in place.
            let mut nodes = serde_json::to_value(&p.nodes).unwrap_or(Value::Null);
            if let Some(r) = s.plugins.borrow().get(id) {
                resolve_paths(&mut nodes, &r.plugin.dir, &paths::storage_dir(id), r.plugin.trust.is_trusted());
            }
            s.link.emit("Patch", json!({"plugin": id, "item": p.item, "monitor": p.monitor, "nodes": nodes}));
        }
        PluginMsg::Menu(m) => {
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.menus.insert((m.item, m.monitor), m.entries);
            }
            publish(s);
        }
        PluginMsg::Popover(p) => {
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                let key = (p.item.clone(), p.monitor.clone());
                match p.view {
                    Some(v) => {
                        r.popovers.insert(key, v);
                    }
                    None => {
                        r.popovers.remove(&key);
                    }
                }
            }
            publish(s);
        }
        PluginMsg::Remove { item, monitor } => {
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.items.retain(|i| !(i.id == item && (monitor.is_none() || i.monitor == monitor)));
            }
            publish(s);
        }
        PluginMsg::Log { level, message } => eprintln!("[{id}] {level:?}: {message}"),
        PluginMsg::FrameReady { .. } | PluginMsg::FrameRequest { .. } => {}
    }
}

pub fn simple(item: &Item) -> Node {
    let mut children = vec![];
    if let Some(i) = &item.icon {
        children.push(Node::new(NodeKind::Icon { name: i.clone(), size: None }).id("icon"));
    }
    if let Some(t) = &item.text {
        children.push(Node::new(NodeKind::Label { text: t.clone(), markup: false }).id("text"));
    }
    if let Some(b) = &item.badge {
        children.push(Node::new(NodeKind::Badge { text: b.clone() }).id("badge"));
    }
    if let Some(p) = item.progress {
        children.push(Node::new(NodeKind::Progress { value: p }).id("progress"));
    }
    let mut n = Node::new(NodeKind::Box { orientation: Orientation::Horizontal, spacing: 6, align: Align::Center, children });
    n.class = item.class.clone();
    n
}

/// A view's file paths as files on this side of the sandbox; anything a third-party
/// plugin may not show is dropped.
fn resolve_paths(v: &mut Value, plugin_dir: &Path, storage_dir: &Path, trusted: bool) {
    let resolve = |p: &str| -> Option<PathBuf> {
        if p.contains("..") {
            return None;
        }
        if let Some(rest) = p.strip_prefix("/plugin/") {
            return Some(plugin_dir.join(rest));
        }
        if let Some(rest) = p.strip_prefix("/storage/") {
            return Some(storage_dir.join(rest));
        }
        if Path::new(p).is_absolute() {
            return trusted.then(|| PathBuf::from(p));
        }
        Some(plugin_dir.join(p))
    };
    match v {
        Value::Object(m) => {
            let node = m.get("node").and_then(Value::as_str).unwrap_or("").to_string();
            if (node == "image" || m.contains_key("path"))
                && let Some(Value::String(p)) = m.get("path").cloned()
            {
                match resolve(&p) {
                    Some(r) => m.insert("path".into(), Value::String(r.display().to_string())),
                    None => m.remove("path"),
                };
            }
            if node == "icon"
                && let Some(Value::String(n)) = m.get("name").cloned()
                && n.contains('/')
            {
                match resolve(&n) {
                    Some(r) => m.insert("name".into(), Value::String(r.display().to_string())),
                    None => m.insert("name".into(), Value::String("image-missing-symbolic".into())),
                };
            }
            for (_, c) in m.iter_mut() {
                resolve_paths(c, plugin_dir, storage_dir, trusted);
            }
        }
        Value::Array(a) => {
            for c in a {
                resolve_paths(c, plugin_dir, storage_dir, trusted);
            }
        }
        _ => {}
    }
}

// ---------- Hiding ----------

pub fn update_hiding(s: &Rc<Shared>) {
    let desktop = s.desktop.borrow().clone();
    let hide_for_fullscreen = s.config.borrow().hide_for_fullscreen;
    let mut any = false;
    let mut changed = false;
    for m in desktop.monitors.iter().filter(|m| !m.disabled) {
        let hide = hide_for_fullscreen && should_hide(&desktop, m);
        if s.hidden.borrow_mut().insert(m.name.clone(), hide) != Some(hide) {
            changed = true;
        }
        any |= !hide;
    }
    if changed {
        publish(s);
    }
    if s.visible.replace(any) != any {
        broadcast(s, |_| Some(HostMsg::Visible { visible: any, item: None }));
    }
}

// ---------- The interface's state ----------

fn key(plugin: &str, item: &str) -> String {
    format!("{plugin}\t{item}")
}

/// The item a monitor shows: its own copy, or the one for every monitor.
fn item_for<'a>(r: &'a Running, item: &str, connector: &str) -> Option<&'a Item> {
    r.items
        .iter()
        .find(|i| i.id == item && i.monitor.as_deref() == Some(connector))
        .or_else(|| r.items.iter().find(|i| i.id == item && i.monitor.is_none()))
}

fn view_value(r: &Running, id: &str, node: &Node) -> Value {
    let mut v = serde_json::to_value(node).unwrap_or(Value::Null);
    resolve_paths(&mut v, &r.plugin.dir, &paths::storage_dir(id), r.plugin.trust.is_trusted());
    v
}

fn item_json(r: &Running, id: &str, i: &Item, connector: &str) -> Value {
    let k = |m: Option<String>| (i.id.clone(), m);
    let view = i.view.clone().unwrap_or_else(|| simple(i));
    let popover = r.popovers.get(&k(Some(connector.into()))).or_else(|| r.popovers.get(&k(None)));
    let menu = r.menus.get(&k(Some(connector.into()))).or_else(|| r.menus.get(&k(None)));
    json!({
        "plugin": id,
        "item": i.id,
        "view": view_value(r, id, &view),
        "rev": r.revs.get(&(i.id.clone(), i.monitor.clone())).copied().unwrap_or(0),
        "tooltip": i.tooltip,
        "visible": i.visible,
        "open": i.open,
        "class": i.class,
        "popover": popover.map(|p| view_value(r, id, p)),
        "menu": menu,
        "click": [i.on_click.is_some(), i.on_middle_click.is_some(), i.on_right_click.is_some()],
    })
}

fn slot_json(slot: &Slot) -> Value {
    let one = |(p, i): &(String, String)| key(p, i);
    match slot {
        Slot::Item(pi) => json!({"kind": "item", "key": one(pi)}),
        Slot::Group(items) => json!({"kind": "group", "keys": items.iter().map(one).collect::<Vec<_>>()}),
        Slot::Divider => json!({"kind": "divider"}),
    }
}

pub fn state_json(s: &Shared) -> Value {
    let desktop = s.desktop.borrow().clone();
    let config = s.config.borrow().clone();
    let plugins = s.plugins.borrow();
    let mut monitors = vec![];
    for m in desktop.monitors.iter().filter(|m| !m.disabled) {
        match &s.only {
            Some(o) if *o != m.name => continue,
            None if !config.enabled_on(m) => continue,
            _ => {}
        }
        let sections = config.sections_for(m);
        let mut shown = vec![];
        let mut items = Map::new();
        for (pid, r) in plugins.iter() {
            let section = r.plugin.manifest.section(Kind::Bar).section.unwrap_or_else(|| "right".into());
            let mut seen = HashSet::new();
            for i in &r.items {
                if i.monitor.as_ref().is_some_and(|x| *x != m.name) || !seen.insert(i.id.clone()) {
                    continue;
                }
                let part = r.plugin.manifest.part_of(Kind::Bar, &i.id).unwrap_or_default();
                shown.push(Shown { plugin: pid.clone(), item: i.id.clone(), part, section: section.clone() });
                if let Some(cur) = item_for(r, &i.id, &m.name) {
                    items.insert(key(pid, &i.id), item_json(r, pid, cur, &m.name));
                }
            }
        }
        let arranged = arrange(&sections, &shown);
        let sections: Vec<Value> = arranged.iter().map(|list| Value::Array(list.iter().map(slot_json).collect())).collect();
        monitors.push(json!({
            "name": m.name,
            "width": m.logical_size().0,
            "hidden": s.hidden.borrow().get(&m.name).copied().unwrap_or(false),
            "sections": sections,
            "items": items,
        }));
    }
    let styles: Map<String, Value> = s.styles.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    json!({
        "size": config.size_name(),
        "height": config.height(),
        "monitors": monitors,
        "styles": styles,
    })
}

// ---------- From the interface ----------

/// Run a command an item asks for, if the plugin may.
fn run_for(s: &Shared, plugin: &str, argv: &[String]) {
    let Some(p) = s.plugins.borrow().get(plugin).map(|r| r.plugin.clone()) else { return };
    if !host::allowed_command(&p, argv) {
        warn_once(s, plugin, format!("it asked to run {:?}, which isn't in its host_commands", argv.join(" ")));
        return;
    }
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).process_group(0);
    match cmd.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(e) => eprintln!("nyx-bar-backend: running {argv:?} for {plugin}: {e}"),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn send_event(
    s: &Shared,
    plugin: &str,
    item: &str,
    connector: &str,
    kind: EventKind,
    button: Option<u32>,
    direction: Option<Direction>,
    control: Option<String>,
    value: Option<Value>,
    anchor: Option<Anchor>,
) {
    if let Some(r) = s.plugins.borrow().get(plugin) {
        r.handle.send(HostMsg::Event(Event { item: item.into(), kind, monitor: Some(connector.into()), button, direction, control, value, anchor }));
    }
}

/// A click on an item: its command if it has one for that button, else the plugin hears
/// it. The interface opens the popover or menu itself (and says so with `PopoverOpened`).
pub fn click(s: &Shared, plugin: &str, item: &str, connector: &str, button: u32, anchor: Option<Anchor>) -> &'static str {
    let (cmd, has_popover, has_menu) = {
        let plugins = s.plugins.borrow();
        let Some(r) = plugins.get(plugin) else { return "none" };
        let Some(i) = item_for(r, item, connector) else { return "none" };
        let cmd = match button {
            1 => i.on_click.clone(),
            2 => i.on_middle_click.clone(),
            3 => i.on_right_click.clone(),
            _ => None,
        };
        let k = |m: Option<String>| (item.to_string(), m);
        let has_popover = r.popovers.contains_key(&k(Some(connector.into()))) || r.popovers.contains_key(&k(None));
        let has_menu = r.menus.contains_key(&k(Some(connector.into()))) || r.menus.contains_key(&k(None));
        (cmd, has_popover, has_menu)
    };
    if let Some(c) = cmd {
        run_for(s, plugin, &c);
        return "command";
    }
    if button == 1 && has_popover {
        return "popover";
    }
    if button == 3 && has_menu {
        return "menu";
    }
    send_event(s, plugin, item, connector, EventKind::Click, Some(button), None, None, None, anchor);
    "event"
}

pub fn scroll(s: &Shared, plugin: &str, item: &str, connector: &str, up: bool) {
    let cmd = s.plugins.borrow().get(plugin).and_then(|r| {
        let i = item_for(r, item, connector)?;
        if up { i.on_scroll_up.clone() } else { i.on_scroll_down.clone() }
    });
    match cmd {
        Some(c) => run_for(s, plugin, &c),
        None => send_event(s, plugin, item, connector, EventKind::Scroll, None, Some(if up { Direction::Up } else { Direction::Down }), None, None, None),
    }
}

/// For `Bar1.ListPlugins`.
pub fn list(s: &Shared) -> Value {
    let plugins = s.plugins.borrow();
    let state = s.state.borrow();
    let mut out = vec![];
    for p in discovery::discover().plugins.into_iter().filter(|p| p.manifest.has_kind(Kind::Bar)) {
        let id = p.manifest.id.clone();
        let r = plugins.get(&id);
        let (memory, cpu) = r.and_then(|r| r.handle.usage()).map(|(m, c)| (Some(m), c)).unwrap_or((None, None));
        let status = match r.map(|r| &r.state) {
            Some(RunState::Running { .. }) => "running".to_string(),
            Some(RunState::Starting) => "starting".to_string(),
            Some(RunState::Crashed(why)) => format!("restarting ({why})"),
            Some(RunState::Unavailable(why)) => format!("off: {why}"),
            None if state.suspended.contains_key(&id) => format!("off: {}", state.suspended[&id].reason),
            None if !nyx_plugin::authorized(&p, &state) => "needs approval".to_string(),
            None => "off".to_string(),
        };
        out.push(json!({
            "id": id,
            "status": status,
            "pid": r.and_then(|r| r.handle.pid()),
            "sandboxed": r.map(|r| r.handle.sandboxed).unwrap_or(!p.trust.is_trusted()),
            "scoped": r.map(|r| r.handle.scoped).unwrap_or(false),
            "restarts": r.map(|r| r.handle.restarts()).unwrap_or(0),
            "memory_bytes": memory,
            "cpu_percent": cpu,
            "items": r.map(|r| r.items.len()).unwrap_or(0),
            "parts": r.map(|r| r.parts.clone()).unwrap_or_default(),
        }));
    }
    Value::Array(out)
}

pub fn restart(s: &Rc<Shared>, id: &str) -> bool {
    let exists = s.plugins.borrow().contains_key(id);
    if exists {
        s.changed_plugins.borrow_mut().insert(id.to_string());
        sync_plugins(s);
    }
    exists
}

pub fn anchor_of(monitor: &str, b: &Value) -> Option<Anchor> {
    let a = b.get("anchor")?;
    let n = |k: &str| a.get(k).and_then(Value::as_f64).unwrap_or(0.0).round() as i32;
    Some(Anchor { monitor: monitor.into(), x: n("x"), y: n("y"), width: n("w"), height: n("h"), edge: "top".into() })
}

