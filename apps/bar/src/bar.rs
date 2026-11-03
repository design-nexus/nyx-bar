//! The bar: one layer-shell window per monitor, and the plugins that fill it.
//!
//! Plugins are found in the three plugin folders, run when the user enabled a bar part
//! (and, for third-party plugins, approved their permissions), and restarted when
//! their folder, manifest, chosen parts or limits change. Their items are drawn by
//! `nyx-plugin-view` and placed by `bar.toml`.

use crate::dbus;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use nyx_bar_core::{Config, Shown, arrange, should_hide};
use nyx_desktop::{Desktop, watch};
use nyx_plugin::{Kind, Plugin, State, discovery, paths};
use nyx_plugin_host::{self as host, Handle, Report, Spec, frames::Buffers, limits::Limits};
use nyx_plugin_protocol::{
    API_VERSION, Anchor, DesktopMonitor, DesktopState, DesktopWorkspace, Direction, Event, EventKind, Hello, HostMsg, Item,
    MenuEntry, MonitorInfo, Node, PluginMsg, ThemeInfo,
};
use nyx_plugin_view::{self as view, Rendered, ViewEvent};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
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
    settings: serde_json::Value,
    limits: Limits,
    /// Items in the order they first came.
    items: Vec<Item>,
    menus: HashMap<Key, Vec<MenuEntry>>,
    popovers: HashMap<Key, Node>,
    frames: HashMap<(String, String), Buffers>,
    pub state: RunState,
    generation: u64,
    warned: HashSet<String>,
}

struct ItemWidget {
    root: gtk::Box,
    rendered: RefCell<Option<Rc<Rendered>>>,
    last: RefCell<Option<Item>>,
    popover: RefCell<Option<(gtk::Popover, Rc<Rendered>)>>,
    menu: RefCell<Option<gtk::PopoverMenu>>,
}

struct Bar {
    connector: String,
    gmon: gdk::Monitor,
    window: gtk::Window,
    boxes: [gtk::Box; 3],
    widgets: RefCell<HashMap<(String, String), Rc<ItemWidget>>>,
    hidden: Cell<bool>,
}

pub struct Shared {
    app: gtk::Application,
    only: Option<String>,
    config: RefCell<Config>,
    desktop: RefCell<Arc<Desktop>>,
    state: RefCell<State>,
    pub plugins: RefCell<BTreeMap<String, Running>>,
    bars: RefCell<Vec<Rc<Bar>>>,
    css: RefCell<HashMap<String, gtk::CssProvider>>,
    user_css: gtk::CssProvider,
    visible: Cell<bool>,
    generation: Cell<u64>,
    sync_pending: Cell<bool>,
    monitors: RefCell<Vec<gio::FileMonitor>>,
    plugin_monitors: RefCell<HashMap<String, gio::FileMonitor>>,
    changed_plugins: RefCell<HashSet<String>>,
    _dbus: RefCell<Option<dbus::Registration>>,
}

fn debug(f: impl FnOnce() -> String) {
    if std::env::var_os("NYX_BAR_DEBUG").is_some() {
        eprintln!("nyx-bar: {}", f());
    }
}

impl Shared {
    fn host_name(&self) -> &'static str {
        if self.only.is_some() { "bar-test" } else { "bar" }
    }

    fn height(&self) -> i32 {
        self.config.borrow().height
    }
}

pub fn start(app: &gtk::Application, only: Option<String>) {
    let path = Config::path();
    if !path.exists() {
        let _ = paths::write_atomic(&path, nyx_plugin::layout::DEFAULT_BAR);
    }
    let config = Config::load(&path).unwrap_or_else(|e| {
        eprintln!("nyx-bar: {e}; using the default layout");
        Config::default()
    });
    let s = Rc::new(Shared {
        app: app.clone(),
        only,
        config: RefCell::new(config),
        desktop: RefCell::new(Arc::new(Desktop::default())),
        state: RefCell::new(State::load()),
        plugins: RefCell::default(),
        bars: RefCell::default(),
        css: RefCell::default(),
        user_css: gtk::CssProvider::new(),
        visible: Cell::new(true),
        generation: Cell::new(0),
        sync_pending: Cell::new(false),
        monitors: RefCell::default(),
        plugin_monitors: RefCell::default(),
        changed_plugins: RefCell::default(),
        _dbus: RefCell::default(),
    });
    std::mem::forget(app.hold());
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &s.user_css, gtk::STYLE_PROVIDER_PRIORITY_USER);
    }
    load_user_css(&s);

    let ws = Rc::downgrade(&s);
    std::mem::forget(watch::watch_local(move |u| {
        let Some(s) = ws.upgrade() else { return };
        debug(|| format!("hyprland: {:?}, connected {}", u.changes, u.desktop.connected));
        *s.desktop.borrow_mut() = u.desktop.clone();
        if u.changes.monitors || s.bars.borrow().is_empty() {
            rebuild_bars(&s);
            let monitors = monitor_infos(&s);
            broadcast(&s, |_| Some(HostMsg::Monitors { monitors: monitors.clone() }));
        }
        if u.changes.windows || u.changes.workspaces || u.changes.active || u.changes.monitors {
            update_hiding(&s);
        }
        if u.changes.workspaces || u.changes.active || u.changes.focus || u.changes.titles || u.changes.windows {
            let state = desktop_state(&u.desktop);
            broadcast(&s, |r| r.plugin.manifest.permissions.desktop.then(|| HostMsg::Desktop(state.clone())));
        }
    }));

    if let Some(display) = gdk::Display::default() {
        let ws = Rc::downgrade(&s);
        display.monitors().connect_items_changed(move |_, _, _, _| {
            let ws = ws.clone();
            glib::timeout_add_local_once(Duration::from_millis(300), move || {
                if let Some(s) = ws.upgrade() {
                    rebuild_bars(&s);
                }
            });
        });
    }

    watch_files(&s);
    let ws = Rc::downgrade(&s);
    nyx_theme::on_change(move || {
        if let Some(s) = ws.upgrade() {
            let theme = theme_info();
            broadcast(&s, |_| Some(HostMsg::Theme { theme: theme.clone() }));
            for b in s.bars.borrow().iter() {
                for w in b.widgets.borrow().values() {
                    w.last.borrow_mut().take();
                }
            }
            relayout_all(&s);
        }
    });

    if let Some(conn) = app.dbus_connection() {
        match dbus::serve(&conn, &s) {
            Ok(r) => *s._dbus.borrow_mut() = Some(r),
            Err(e) => eprintln!("nyx-bar: couldn't serve Bar1 on D-Bus: {e}"),
        }
    }
    sync_plugins(&s);
    // Everything else holds the bar weakly; it lives as long as the app.
    std::mem::forget(s);
}

// ---------- Files ----------

fn monitor_file(path: &std::path::Path, f: impl Fn() + 'static) -> Option<gio::FileMonitor> {
    let file = gio::File::for_path(path);
    let m = file.monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE).ok()?;
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

fn monitor_dir(path: &std::path::Path, f: impl Fn(Option<String>) + 'static) -> Option<gio::FileMonitor> {
    let _ = std::fs::create_dir_all(path);
    let m = gio::File::for_path(path).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE).ok()?;
    m.connect_changed(move |_, file, _, _| f(file.basename().map(|b| b.to_string_lossy().into_owned())));
    Some(m)
}

fn watch_files(s: &Rc<Shared>) {
    let mut monitors = vec![];
    let ws = Rc::downgrade(s);
    monitors.extend(monitor_file(&Config::path(), move || {
        let Some(s) = ws.upgrade() else { return };
        match Config::load(&Config::path()) {
            Ok(c) => {
                let height_changed = c.height != s.height();
                *s.config.borrow_mut() = c;
                if height_changed {
                    for b in s.bars.borrow_mut().drain(..) {
                        b.window.destroy();
                    }
                    rebuild_bars(&s);
                }
                schedule_sync(&s);
            }
            Err(e) => eprintln!("nyx-bar: {e}; keeping the layout as it was"),
        }
    }));
    let ws = Rc::downgrade(s);
    monitors.extend(monitor_file(&paths::state_file(), move || {
        if let Some(s) = ws.upgrade() {
            schedule_sync(&s);
        }
    }));
    let ws = Rc::downgrade(s);
    monitors.extend(monitor_file(&paths::config_home().join("nyx/bar.css"), move || {
        if let Some(s) = ws.upgrade() {
            load_user_css(&s);
        }
    }));
    for (dir, _) in discovery::search_dirs() {
        let ws = Rc::downgrade(s);
        monitors.extend(monitor_dir(&dir, move |name| {
            let Some(s) = ws.upgrade() else { return };
            if name.as_deref().is_some_and(|n| n.starts_with('.')) {
                return;
            }
            schedule_sync(&s);
        }));
    }
    *s.monitors.borrow_mut() = monitors;
}

fn load_user_css(s: &Shared) {
    let path = paths::config_home().join("nyx/bar.css");
    s.user_css.load_from_string(&std::fs::read_to_string(path).unwrap_or_default());
}

/// Watch each running plugin's folder: an edit restarts it.
fn watch_plugin_dirs(s: &Rc<Shared>) {
    let running: Vec<(String, std::path::PathBuf)> =
        s.plugins.borrow().iter().map(|(id, r)| (id.clone(), r.plugin.dir.clone())).collect();
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

fn schedule_sync(s: &Rc<Shared>) {
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

// ---------- Plugins ----------

fn theme_info() -> ThemeInfo {
    ThemeInfo { light: nyx_theme::palette().light, name: nyx_theme::current_theme().id }
}

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
        monitors: d
            .monitors
            .iter()
            .map(|m| DesktopMonitor { name: m.name.clone(), active_workspace: m.active_workspace, focused: m.focused })
            .collect(),
        workspaces: d
            .workspaces
            .iter()
            .filter(|w| w.is_user())
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
    for p in &found.problems {
        debug(|| format!("{}: {}", p.dir.display(), p.error));
    }
    let state = State::load();
    let changed: HashSet<String> = s.changed_plugins.take();
    let wanted: Vec<Plugin> = found.plugins.into_iter().filter(|p| p.manifest.has_kind(Kind::Bar) && state.runs(p, Kind::Bar)).collect();
    *s.state.borrow_mut() = state.clone();
    let config = s.config.borrow().clone();
    let stale: Vec<String> = s.plugins.borrow().keys().filter(|id| !wanted.iter().any(|p| p.manifest.id == **id)).cloned().collect();
    for id in stale {
        debug(|| format!("stopping {id}"));
        s.plugins.borrow_mut().remove(&id);
        if let Some(a) = s.css.borrow_mut().remove(&id) { remove_provider(a) }
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
    relayout_all(s);
}

fn remove_provider(p: gtk::CssProvider) {
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_remove_provider_for_display(&display, &p);
    }
}

fn load_plugin_css(s: &Shared, p: &Plugin, limits: &Limits) {
    if let Some(old) = s.css.borrow_mut().remove(&p.manifest.id) {
        remove_provider(old);
    }
    let Ok(text) = std::fs::read_to_string(p.dir.join("style.css")) else { return };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&view::css::scope(&text, &p.manifest.id, limits.infinite_animations));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    }
    s.css.borrow_mut().insert(p.manifest.id.clone(), provider);
}

fn spawn_plugin(s: &Rc<Shared>, p: Plugin, parts: Vec<String>, settings: serde_json::Value, limits: Limits) {
    let id = p.manifest.id.clone();
    load_plugin_css(s, &p, &limits);
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
        bar_height: Some(s.height()),
    };
    let spec = Spec { plugin: p.clone(), kind: Kind::Bar, host: s.host_name().into(), hello, limits: limits.clone() };
    let (tx, rx) = async_channel::unbounded();
    let handle = host::spawn(spec, tx);
    let generation = s.generation.get() + 1;
    s.generation.set(generation);
    if !s.visible.get() {
        handle.send(HostMsg::Visible { visible: false, item: None });
    }
    // Replacing an old run drops its handle, which stops it.
    s.plugins.borrow_mut().insert(
        id.clone(),
        Running {
            plugin: p,
            handle,
            parts,
            settings,
            limits,
            items: vec![],
            menus: HashMap::new(),
            popovers: HashMap::new(),
            frames: HashMap::new(),
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
        eprintln!("nyx-bar: {id}: {what}");
    }
}

fn on_report(s: &Rc<Shared>, id: &str, report: Report) {
    match report {
        Report::Started { pid, scoped, sandboxed } => {
            debug(|| format!("{id} started (pid {pid}, scope {scoped}, sandbox {sandboxed})"));
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.state = RunState::Running { pid, scoped, sandboxed };
            }
        }
        Report::Message(m) => on_message(s, id, m),
        Report::Frames(b) => {
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.frames.insert((b.item.clone(), b.node.clone()), b);
            }
        }
        Report::Exited { status, stderr } => {
            eprintln!("nyx-bar: {id} stopped ({status}){}", if stderr.is_empty() { String::new() } else { format!(": {stderr}") });
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.state = RunState::Crashed(status);
                r.items.clear();
                r.frames.clear();
            }
            relayout_all(s);
        }
        Report::Suspended { reason, crashed } => {
            let name = s.plugins.borrow().get(id).map(|r| r.plugin.manifest.name.clone()).unwrap_or_else(|| id.to_string());
            eprintln!("nyx-bar: {id} is off: {reason}");
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.state = RunState::Unavailable(reason.clone());
                r.items.clear();
            }
            relayout_all(s);
            if crashed {
                if let Ok(mut e) = nyx_plugin::state::Editor::open() {
                    e.suspend(id, &reason, &nyx_plugin::state::now_utc());
                    let _ = e.save();
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
            let new = {
                let mut plugins = s.plugins.borrow_mut();
                let Some(r) = plugins.get_mut(id) else { return };
                let part = r.plugin.manifest.part_of(Kind::Bar, &item.id);
                if !part.as_ref().is_some_and(|p| r.parts.contains(p)) {
                    let what = format!("item {} isn't one of the parts turned on ({})", item.id, r.parts.join(", "));
                    drop(plugins);
                    warn_once(s, id, what);
                    return;
                }
                match r.items.iter_mut().find(|i| i.id == item.id && i.monitor == item.monitor) {
                    Some(old) => {
                        *old = item.clone();
                        false
                    }
                    None => {
                        r.items.push(item.clone());
                        true
                    }
                }
            };
            if new {
                relayout_all(s);
            } else {
                for b in s.bars.borrow().iter() {
                    if item.monitor.as_ref().is_none_or(|m| *m == b.connector) {
                        refresh_widget(s, b, id, &item.id);
                    }
                }
            }
        }
        PluginMsg::Patch(p) => {
            {
                let mut plugins = s.plugins.borrow_mut();
                let Some(r) = plugins.get_mut(id) else { return };
                for item in r.items.iter_mut().filter(|i| i.id == p.item && (p.monitor.is_none() || i.monitor == p.monitor)) {
                    let mut view = item.view.take().unwrap_or_else(|| view::simple(item));
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
            let latest: Vec<Item> = s.plugins.borrow().get(id).map(|r| r.items.iter().filter(|i| i.id == p.item).cloned().collect()).unwrap_or_default();
            for b in s.bars.borrow().iter() {
                if p.monitor.as_ref().is_some_and(|m| *m != b.connector) {
                    continue;
                }
                let Some(w) = b.widgets.borrow().get(&(id.to_string(), p.item.clone())).cloned() else { continue };
                let ctx = item_ctx(s, id, &p.item, &b.connector, None);
                if let (Some(r), Some(ctx)) = (w.rendered.borrow().as_ref(), ctx.as_ref()) {
                    for n in &p.nodes {
                        r.patch(n, ctx);
                    }
                }
                if let (Some((_, r)), Some(ctx)) = (w.popover.borrow().as_ref(), ctx.as_ref()) {
                    for n in &p.nodes {
                        r.patch(n, ctx);
                    }
                }
                // The stored item already has the patch; don't draw it again for it.
                if let Some(i) = latest.iter().find(|i| i.monitor.as_ref().is_none_or(|m| *m == b.connector)) {
                    *w.last.borrow_mut() = Some(i.clone());
                }
            }
        }
        PluginMsg::Menu(m) => {
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.menus.insert((m.item, m.monitor), m.entries);
            }
        }
        PluginMsg::Popover(p) => {
            let key = (p.item.clone(), p.monitor.clone());
            match p.view {
                Some(v) => {
                    if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                        r.popovers.insert(key, v);
                    }
                }
                None => {
                    if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                        r.popovers.remove(&key);
                    }
                }
            }
            for b in s.bars.borrow().iter() {
                if let Some(w) = b.widgets.borrow().get(&(id.to_string(), p.item.clone())).cloned()
                    && w.popover.borrow().as_ref().is_some_and(|(pop, _)| pop.is_visible())
                {
                    fill_popover(s, b, &w, id, &p.item);
                }
            }
        }
        PluginMsg::Remove { item, monitor } => {
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.items.retain(|i| !(i.id == item && (monitor.is_none() || i.monitor == monitor)));
            }
            relayout_all(s);
        }
        PluginMsg::Log { level, message } => eprintln!("[{id}] {level:?}: {message}"),
        PluginMsg::FrameReady { item, node, buffer } => {
            if !s.visible.get() {
                return;
            }
            let Some(b) = s.plugins.borrow().get(id).and_then(|r| r.frames.get(&(item.clone(), node.clone())).cloned()) else { return };
            let Some(map) = b.maps.get(buffer as usize) else { return };
            for bar in s.bars.borrow().iter() {
                if let Some(w) = bar.widgets.borrow().get(&(id.to_string(), item.clone()))
                    && let Some(r) = w.rendered.borrow().as_ref()
                    && let Some(pic) = r.frame(&node)
                {
                    view::show_frame(&pic, map.bytes(), b.width, b.height, b.stride);
                }
            }
        }
        PluginMsg::FrameRequest { .. } => {}
    }
}

// ---------- Bars ----------

fn rebuild_bars(s: &Rc<Shared>) {
    let desktop = s.desktop.borrow().clone();
    let Some(display) = gdk::Display::default() else { return };
    let gmons: Vec<gdk::Monitor> = display.monitors().iter::<gdk::Monitor>().flatten().collect();
    let config = s.config.borrow().clone();
    let old: Vec<Rc<Bar>> = s.bars.borrow_mut().drain(..).collect();
    debug(|| {
        let names: Vec<String> = gmons.iter().map(|g| g.connector().map(|c| c.to_string()).unwrap_or_default()).collect();
        format!("rebuild: hyprland {:?}, gdk {names:?}", desktop.monitors.iter().map(|m| &m.name).collect::<Vec<_>>())
    });
    let mut bars = vec![];
    for m in desktop.monitors.iter().filter(|m| !m.disabled) {
        match &s.only {
            Some(o) if *o != m.name => continue,
            None if !config.enabled_on(m) => continue,
            _ => {}
        }
        let Some(gmon) = gmons.iter().find(|g| g.connector().as_deref() == Some(m.name.as_str())) else { continue };
        if let Some(b) = old.iter().find(|b| b.connector == m.name && b.gmon == *gmon) {
            bars.push(b.clone());
            continue;
        }
        debug(|| format!("bar on {}", m.name));
        bars.push(new_bar(s, &m.name, gmon));
    }
    for b in old.iter().filter(|b| !bars.iter().any(|k| Rc::ptr_eq(k, b))) {
        b.window.destroy();
    }
    *s.bars.borrow_mut() = bars;
    relayout_all(s);
    update_hiding(s);
}

fn new_bar(s: &Rc<Shared>, connector: &str, gmon: &gdk::Monitor) -> Rc<Bar> {
    let height = s.height();
    let window = gtk::Window::new();
    window.set_application(Some(&s.app));
    window.add_css_class("nyx-bar");
    window.set_decorated(false);
    window.init_layer_shell();
    window.set_namespace(Some("nyx-bar"));
    window.set_layer(Layer::Top);
    window.set_monitor(Some(gmon));
    window.set_keyboard_mode(KeyboardMode::None);
    for e in [Edge::Top, Edge::Left, Edge::Right] {
        window.set_anchor(e, true);
    }
    window.set_exclusive_zone(height);
    window.set_default_size(-1, height);
    window.set_size_request(-1, height);
    let center = gtk::CenterBox::new();
    center.add_css_class("bar-sections");
    let boxes = ["bar-left", "bar-center", "bar-right"].map(|c| {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        b.add_css_class(c);
        b.set_valign(gtk::Align::Fill);
        b
    });
    center.set_start_widget(Some(&boxes[0]));
    center.set_center_widget(Some(&boxes[1]));
    center.set_end_widget(Some(&boxes[2]));
    window.set_child(Some(&center));
    window.present();
    Rc::new(Bar { connector: connector.into(), gmon: gmon.clone(), window, boxes, widgets: RefCell::default(), hidden: Cell::new(false) })
}

fn update_hiding(s: &Rc<Shared>) {
    let desktop = s.desktop.borrow().clone();
    let hide_for_fullscreen = s.config.borrow().hide_for_fullscreen;
    let mut any = false;
    for b in s.bars.borrow().iter() {
        let hide = hide_for_fullscreen && desktop.monitor(&b.connector).is_some_and(|m| should_hide(&desktop, m));
        if b.hidden.replace(hide) != hide {
            debug(|| format!("{} {}", if hide { "hiding on" } else { "showing on" }, b.connector));
            b.window.set_visible(!hide);
        }
        any |= !hide;
    }
    if s.visible.replace(any) != any {
        broadcast(s, |_| Some(HostMsg::Visible { visible: any, item: None }));
    }
}

fn relayout_all(s: &Rc<Shared>) {
    for b in s.bars.borrow().clone() {
        relayout(s, &b);
    }
}

/// The item a bar shows: its monitor's copy, or the one for every monitor.
fn item_for<'a>(r: &'a Running, item: &str, connector: &str) -> Option<&'a Item> {
    r.items
        .iter()
        .find(|i| i.id == item && i.monitor.as_deref() == Some(connector))
        .or_else(|| r.items.iter().find(|i| i.id == item && i.monitor.is_none()))
}

fn relayout(s: &Rc<Shared>, b: &Rc<Bar>) {
    let desktop = s.desktop.borrow().clone();
    let Some(monitor) = desktop.monitor(&b.connector).cloned() else { return };
    let sections = s.config.borrow().sections_for(&monitor);
    let shown: Vec<Shown> = {
        let plugins = s.plugins.borrow();
        let mut out = vec![];
        for (pid, r) in plugins.iter() {
            let section = r.plugin.manifest.section(Kind::Bar).section.unwrap_or_else(|| "right".into());
            let mut seen = HashSet::new();
            for i in &r.items {
                if i.monitor.as_ref().is_some_and(|m| *m != b.connector) || !seen.insert(i.id.clone()) {
                    continue;
                }
                let part = r.plugin.manifest.part_of(Kind::Bar, &i.id).unwrap_or_default();
                out.push(Shown { plugin: pid.clone(), item: i.id.clone(), part, section: section.clone() });
            }
        }
        out
    };
    let arranged = arrange(&sections, &shown);
    let mut keep = HashSet::new();
    for (bx, list) in b.boxes.iter().zip(arranged.iter()) {
        while let Some(c) = bx.first_child() {
            bx.remove(&c);
        }
        for (plugin, item) in list {
            let key = (plugin.clone(), item.clone());
            let existing = b.widgets.borrow().get(&key).cloned();
            let w = match existing {
                Some(w) => w,
                None => {
                    let w = new_item_widget(s, b, plugin, item);
                    b.widgets.borrow_mut().insert(key.clone(), w.clone());
                    w
                }
            };
            refresh_widget(s, b, plugin, item);
            bx.append(&w.root);
            keep.insert(key);
        }
    }
    b.widgets.borrow_mut().retain(|k, w| {
        let kept = keep.contains(k);
        if !kept {
            if let Some((p, _)) = w.popover.borrow_mut().take() {
                p.unparent();
            }
            if let Some(m) = w.menu.borrow_mut().take() {
                m.unparent();
            }
        }
        kept
    });
}

fn item_ctx(s: &Rc<Shared>, plugin: &str, item: &str, connector: &str, max_height: Option<Option<i32>>) -> Option<view::Ctx> {
    let plugins = s.plugins.borrow();
    let r = plugins.get(plugin)?;
    let (ws, pid, iid, conn) = (Rc::downgrade(s), plugin.to_string(), item.to_string(), connector.to_string());
    Some(view::Ctx {
        plugin: plugin.to_string(),
        plugin_dir: r.plugin.dir.clone(),
        storage_dir: paths::storage_dir(plugin),
        trusted: r.plugin.trust.is_trusted(),
        max_height: max_height.unwrap_or(Some(s.height() - 8)),
        on_event: Rc::new(move |e: ViewEvent| {
            if let Some(s) = ws.upgrade() {
                send_event(&s, &pid, &iid, &conn, e.kind, None, None, e.control, e.value, None);
            }
        }),
    })
}

#[allow(clippy::too_many_arguments)]
fn send_event(
    s: &Shared,
    plugin: &str,
    item: &str,
    connector: &str,
    kind: EventKind,
    button: Option<u32>,
    direction: Option<Direction>,
    control: Option<String>,
    value: Option<serde_json::Value>,
    anchor: Option<Anchor>,
) {
    if let Some(r) = s.plugins.borrow().get(plugin) {
        r.handle.send(HostMsg::Event(Event { item: item.into(), kind, monitor: Some(connector.into()), button, direction, control, value, anchor }));
    }
}

fn refresh_widget(s: &Rc<Shared>, b: &Rc<Bar>, plugin: &str, item: &str) {
    let Some(w) = b.widgets.borrow().get(&(plugin.to_string(), item.to_string())).cloned() else { return };
    let Some(current) = s.plugins.borrow().get(plugin).and_then(|r| item_for(r, item, &b.connector).cloned()) else { return };
    w.root.set_visible(current.visible);
    w.root.set_tooltip_text(current.tooltip.as_deref());
    if w.last.borrow().as_ref() == Some(&current) {
        return;
    }
    let Some(ctx) = item_ctx(s, plugin, item, &b.connector, None) else { return };
    let node = current.view.clone().unwrap_or_else(|| view::simple(&current));
    let rendered = Rc::new(view::render(&node, &ctx));
    rendered.attach_states(w.root.upcast_ref(), &ctx);
    while let Some(c) = w.root.first_child() {
        w.root.remove(&c);
    }
    w.root.append(&rendered.root);
    for c in w.root.css_classes() {
        if c.starts_with("item-class-") {
            w.root.remove_css_class(&c);
        }
    }
    for c in &current.class {
        if c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
            w.root.add_css_class(c);
        }
    }
    *w.rendered.borrow_mut() = Some(rendered);
    *w.last.borrow_mut() = Some(current);
}

fn new_item_widget(s: &Rc<Shared>, b: &Rc<Bar>, plugin: &str, item: &str) -> Rc<ItemWidget> {
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    root.add_css_class("bar-item");
    root.add_css_class(&view::css::plugin_class(plugin));
    let safe: String = item.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    root.add_css_class(&format!("item-{safe}"));
    root.set_valign(gtk::Align::Fill);
    let w = Rc::new(ItemWidget {
        root: root.clone(),
        rendered: RefCell::default(),
        last: RefCell::default(),
        popover: RefCell::default(),
        menu: RefCell::default(),
    });
    let click = gtk::GestureClick::new();
    click.set_button(0);
    let (ws, wb, ww, pid, iid) = (Rc::downgrade(s), Rc::downgrade(b), Rc::downgrade(&w), plugin.to_string(), item.to_string());
    click.connect_released(move |g, _, _, _| {
        let (Some(s), Some(b), Some(w)) = (ws.upgrade(), wb.upgrade(), ww.upgrade()) else { return };
        on_click(&s, &b, &w, &pid, &iid, g.current_button());
    });
    root.add_controller(click);
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE);
    let (ws, wb, pid, iid) = (Rc::downgrade(s), Rc::downgrade(b), plugin.to_string(), item.to_string());
    scroll.connect_scroll(move |_, _, dy| {
        let (Some(s), Some(b)) = (ws.upgrade(), wb.upgrade()) else { return glib::Propagation::Proceed };
        let up = dy < 0.0;
        let cmd = s.plugins.borrow().get(&pid).and_then(|r| {
            let i = item_for(r, &iid, &b.connector)?;
            if up { i.on_scroll_up.clone() } else { i.on_scroll_down.clone() }
        });
        match cmd {
            Some(c) => run_for(&s, &pid, &c),
            None => send_event(&s, &pid, &iid, &b.connector, EventKind::Scroll, None, Some(if up { Direction::Up } else { Direction::Down }), None, None, None),
        }
        glib::Propagation::Stop
    });
    root.add_controller(scroll);
    w
}

fn anchor(b: &Bar, w: &gtk::Widget) -> Anchor {
    let r = w.compute_bounds(&b.window);
    let (x, y, width, height) = r.map(|r| (r.x() as i32, r.y() as i32, r.width() as i32, r.height() as i32)).unwrap_or_default();
    Anchor { monitor: b.connector.clone(), x, y, width, height, edge: "top".into() }
}

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
        Err(e) => eprintln!("nyx-bar: running {argv:?} for {plugin}: {e}"),
    }
}

fn on_click(s: &Rc<Shared>, b: &Rc<Bar>, w: &Rc<ItemWidget>, plugin: &str, item: &str, button: u32) {
    let (cmd, has_popover, menu) = {
        let plugins = s.plugins.borrow();
        let Some(r) = plugins.get(plugin) else { return };
        let Some(i) = item_for(r, item, &b.connector) else { return };
        let cmd = match button {
            1 => i.on_click.clone(),
            2 => i.on_middle_click.clone(),
            3 => i.on_right_click.clone(),
            _ => None,
        };
        let key = |m: Option<String>| (item.to_string(), m);
        let has_popover = r.popovers.contains_key(&key(Some(b.connector.clone()))) || r.popovers.contains_key(&key(None));
        let menu = r.menus.get(&key(Some(b.connector.clone()))).or_else(|| r.menus.get(&key(None))).cloned();
        (cmd, has_popover, menu)
    };
    if let Some(c) = cmd {
        run_for(s, plugin, &c);
        return;
    }
    if button == 1 && has_popover {
        open_popover(s, b, w, plugin, item);
        return;
    }
    if button == 3
        && let Some(entries) = menu
    {
        open_menu(s, b, w, plugin, item, &entries);
        return;
    }
    send_event(s, plugin, item, &b.connector, EventKind::Click, Some(button), None, None, None, Some(anchor(b, w.root.upcast_ref())));
}

fn fill_popover(s: &Rc<Shared>, b: &Rc<Bar>, w: &Rc<ItemWidget>, plugin: &str, item: &str) {
    let node = {
        let plugins = s.plugins.borrow();
        let Some(r) = plugins.get(plugin) else { return };
        r.popovers.get(&(item.to_string(), Some(b.connector.clone()))).or_else(|| r.popovers.get(&(item.to_string(), None))).cloned()
    };
    let (Some(node), Some(ctx)) = (node, item_ctx(s, plugin, item, &b.connector, Some(None))) else { return };
    let limits = s.plugins.borrow().get(plugin).map(|r| r.limits.popover_max).unwrap_or((480, 640));
    let rendered = Rc::new(view::render(&node, &ctx));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_propagate_natural_height(true);
    scroll.set_propagate_natural_width(true);
    scroll.set_max_content_height(limits.1);
    scroll.set_max_content_width(limits.0);
    scroll.set_child(Some(&rendered.root));
    let existing = w.popover.borrow().as_ref().map(|(p, _)| p.clone());
    let pop = existing.unwrap_or_else(|| {
        let p = gtk::Popover::new();
        p.add_css_class("nyx-bar-popover");
        p.set_has_arrow(false);
        p.set_position(gtk::PositionType::Bottom);
        p.set_parent(&w.root);
        let (ws, wb, wr, pid, iid) = (Rc::downgrade(s), Rc::downgrade(b), w.root.downgrade(), plugin.to_string(), item.to_string());
        p.connect_closed(move |_| {
            let (Some(s), Some(b)) = (ws.upgrade(), wb.upgrade()) else { return };
            if let Some(r) = wr.upgrade() {
                r.remove_css_class("open");
            }
            b.window.set_keyboard_mode(KeyboardMode::None);
            send_event(&s, &pid, &iid, &b.connector, EventKind::PopoverClose, None, None, None, None, None);
        });
        p
    });
    pop.set_child(Some(&scroll));
    *w.popover.borrow_mut() = Some((pop, rendered));
}

fn open_popover(s: &Rc<Shared>, b: &Rc<Bar>, w: &Rc<ItemWidget>, plugin: &str, item: &str) {
    fill_popover(s, b, w, plugin, item);
    let Some((pop, _)) = w.popover.borrow().clone() else { return };
    b.window.set_keyboard_mode(KeyboardMode::OnDemand);
    w.root.add_css_class("open");
    pop.popup();
    send_event(s, plugin, item, &b.connector, EventKind::PopoverOpen, Some(1), None, None, None, Some(anchor(b, w.root.upcast_ref())));
}

fn open_menu(s: &Rc<Shared>, b: &Rc<Bar>, w: &Rc<ItemWidget>, plugin: &str, item: &str, entries: &[MenuEntry]) {
    if let Some(old) = w.menu.borrow_mut().take() {
        old.unparent();
    }
    let (ws, pid, iid, conn) = (Rc::downgrade(s), plugin.to_string(), item.to_string(), b.connector.clone());
    let (model, group) = view::menu(
        entries,
        Rc::new(move |id: String| {
            if let Some(s) = ws.upgrade() {
                send_event(&s, &pid, &iid, &conn, EventKind::Action, None, None, Some(id), None, None);
            }
        }),
    );
    w.root.insert_action_group("pm", Some(&group));
    let menu = gtk::PopoverMenu::from_model(Some(&model));
    menu.add_css_class("nyx-bar-popover");
    menu.set_has_arrow(false);
    menu.set_position(gtk::PositionType::Bottom);
    menu.set_parent(&w.root);
    let wb = Rc::downgrade(b);
    menu.connect_closed(move |_| {
        if let Some(b) = wb.upgrade() {
            b.window.set_keyboard_mode(KeyboardMode::None);
        }
    });
    b.window.set_keyboard_mode(KeyboardMode::OnDemand);
    menu.popup();
    *w.menu.borrow_mut() = Some(menu);
}

/// For `Bar1.ListPlugins`.
pub fn list(s: &Shared) -> serde_json::Value {
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
        out.push(serde_json::json!({
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
            "limits": r.map(|r| serde_json::json!({
                "memory_mb": r.limits.memory_mb,
                "cpu_percent": r.limits.cpu_percent,
                "tasks": r.limits.tasks,
                "messages_per_sec": r.limits.messages_per_sec,
            })),
        }));
    }
    serde_json::Value::Array(out)
}

pub fn restart(s: &Rc<Shared>, id: &str) -> bool {
    let exists = s.plugins.borrow().contains_key(id);
    if exists {
        s.changed_plugins.borrow_mut().insert(id.to_string());
        sync_plugins(s);
    }
    exists
}
