//! Draws plugin views with GTK, for every host. A host renders an item's (or a
//! popover's, a tile's) [`Node`] tree with [`render`], gets events back through
//! [`Ctx::on_event`], and applies `patch` messages with [`Rendered::patch`].
//!
//! Colours are theme tokens resolved to the shared stylesheet; sizes, colours and
//! fonts set on nodes become generated CSS classes in one provider, so no plugin can
//! inject CSS through a value. A plugin's own `style.css` is scoped by [`css::scope`].

pub mod color;
pub mod css;
pub mod vector;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use nyx_plugin_protocol::{Align, EventKind, GraphStyle, Item, MenuEntry, Node, NodeKind, NodePatch, Orientation, Transition, Vector};
use serde_json::{Map, Value};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// An event from a view, before the host adds the item and monitor.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewEvent {
    pub kind: EventKind,
    pub control: Option<String>,
    pub value: Option<Value>,
}

/// What rendering needs from the host.
#[derive(Clone)]
pub struct Ctx {
    pub plugin: String,
    /// Where `/plugin/…` and `/storage/…` paths in views point.
    pub plugin_dir: PathBuf,
    pub storage_dir: PathBuf,
    /// Third-party plugins may only show files from those two folders.
    pub trusted: bool,
    /// The largest height inline nodes may take (the bar's height); None in popovers.
    pub max_height: Option<i32>,
    pub on_event: Rc<dyn Fn(ViewEvent)>,
}

impl Ctx {
    fn emit(&self, kind: EventKind, control: Option<String>, value: Option<Value>) {
        (self.on_event)(ViewEvent { kind, control, value })
    }

    /// A path a view names, as a file on this side of the sandbox.
    pub fn resolve_path(&self, p: &str) -> Option<PathBuf> {
        if p.contains("..") {
            return None;
        }
        if let Some(rest) = p.strip_prefix("/plugin/") {
            return Some(self.plugin_dir.join(rest));
        }
        if let Some(rest) = p.strip_prefix("/storage/") {
            return Some(self.storage_dir.join(rest));
        }
        if self.trusted && Path::new(p).is_absolute() {
            return Some(PathBuf::from(p));
        }
        if !p.starts_with('/') {
            return Some(self.plugin_dir.join(p));
        }
        None
    }
}

#[derive(Clone)]
enum Handle {
    Box(gtk::Box),
    Overlay(gtk::Overlay),
    Stack(gtk::Stack),
    Label(gtk::Label),
    Icon(gtk::Image),
    Image(gtk::Picture),
    Badge(gtk::Label),
    Progress(gtk::LevelBar),
    Graph(gtk::DrawingArea, Rc<RefCell<(Vec<f64>, Option<f64>, GraphStyle, Option<String>)>>),
    Spinner(gtk::Spinner),
    Vector(gtk::DrawingArea, Rc<RefCell<Vector>>),
    Frame(gtk::Picture),
    Row(gtk::Button, gtk::Image, gtk::Label, gtk::Label),
    Toggle(gtk::Switch, gtk::Label, Rc<Cell<bool>>),
    Slider(gtk::Scale, Rc<Cell<bool>>),
    Button(gtk::Button),
    Entry(gtk::Entry),
    Separator(gtk::Separator),
    Scroll(gtk::ScrolledWindow),
}

impl Handle {
    fn widget(&self) -> gtk::Widget {
        match self {
            Handle::Box(w) => w.clone().upcast(),
            Handle::Overlay(w) => w.clone().upcast(),
            Handle::Stack(w) => w.clone().upcast(),
            Handle::Label(w) | Handle::Badge(w) => w.clone().upcast(),
            Handle::Icon(w) => w.clone().upcast(),
            Handle::Image(w) | Handle::Frame(w) => w.clone().upcast(),
            Handle::Progress(w) => w.clone().upcast(),
            Handle::Graph(w, _) | Handle::Vector(w, _) => w.clone().upcast(),
            Handle::Spinner(w) => w.clone().upcast(),
            Handle::Row(w, ..) => w.clone().upcast(),
            Handle::Toggle(w, ..) => w.parent().unwrap_or_else(|| w.clone().upcast()),
            Handle::Slider(w, _) => w.parent().unwrap_or_else(|| w.clone().upcast()),
            Handle::Button(w) => w.clone().upcast(),
            Handle::Entry(w) => w.clone().upcast(),
            Handle::Separator(w) => w.clone().upcast(),
            Handle::Scroll(w) => w.clone().upcast(),
        }
    }
}

/// A rendered tree.
pub struct Rendered {
    pub root: gtk::Widget,
    nodes: HashMap<String, (Handle, Vec<String>)>,
    /// Nodes with hover or pressed properties: (handle, its classes, props, originals).
    states: Vec<StateProps>,
}

struct StateProps {
    handle: Handle,
    classes: Rc<RefCell<Vec<String>>>,
    hover: Option<Map<String, Value>>,
    pressed: Option<Map<String, Value>>,
    original: Map<String, Value>,
}

thread_local! {
    static DYNAMIC: RefCell<Option<(gtk::CssProvider, HashSet<String>, String)>> = const { RefCell::new(None) };
}

/// A class for one CSS declaration set, defined once in a shared provider.
fn style_class(decls: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in decls.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let class = format!("nv-{h:x}");
    DYNAMIC.with(|d| {
        let mut d = d.borrow_mut();
        let (provider, known, text) = d.get_or_insert_with(|| {
            let p = gtk::CssProvider::new();
            if let Some(display) = gdk::Display::default() {
                gtk::style_context_add_provider_for_display(&display, &p, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2);
            }
            (p, HashSet::new(), String::new())
        });
        if known.insert(class.clone()) {
            text.push_str(&format!(".{class} {{ {decls} }}\n"));
            provider.load_from_string(text);
        }
    });
    class
}

/// CSS for a node's look properties (from the node or a patch).
fn look_decls(props: &Map<String, Value>, max_height: Option<i32>) -> String {
    let mut d = String::new();
    if let Some(c) = props.get("color").and_then(Value::as_str).and_then(color::css) {
        d.push_str(&format!("color: {c}; "));
    }
    if let Some(c) = props.get("background").and_then(Value::as_str).and_then(color::css) {
        d.push_str(&format!("background-color: {c}; border-radius: 7px; "));
    }
    if let Some(f) = props.get("font_size").and_then(Value::as_f64) {
        let f = match max_height {
            Some(h) => f.clamp(6.0, h as f64 * 0.75),
            None => f.clamp(6.0, 96.0),
        };
        d.push_str(&format!("font-size: {f:.1}px; "));
    }
    if let Some(r) = props.get("radius").and_then(Value::as_f64) {
        d.push_str(&format!("border-radius: {:.1}px; ", r.clamp(0.0, 999.0)));
    }
    d
}

fn set_look(w: &gtk::Widget, classes: &Rc<RefCell<Vec<String>>>, props: &Map<String, Value>, max_height: Option<i32>) {
    let decls = look_decls(props, max_height);
    let mut cs = classes.borrow_mut();
    for c in cs.drain(..) {
        w.remove_css_class(&c);
    }
    if !decls.is_empty() {
        let c = style_class(&decls);
        w.add_css_class(&c);
        cs.push(c);
    }
}

fn node_props(node: &Node) -> Map<String, Value> {
    match serde_json::to_value(node) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn align(a: Align) -> gtk::Align {
    match a {
        Align::Center => gtk::Align::Center,
        Align::Start => gtk::Align::Start,
        Align::End => gtk::Align::End,
        Align::Fill => gtk::Align::Fill,
    }
}

fn transition(t: Transition) -> gtk::StackTransitionType {
    match t {
        Transition::Crossfade => gtk::StackTransitionType::Crossfade,
        Transition::SlideUp => gtk::StackTransitionType::SlideUp,
        Transition::SlideDown => gtk::StackTransitionType::SlideDown,
        Transition::SlideLeft => gtk::StackTransitionType::SlideLeft,
        Transition::SlideRight => gtk::StackTransitionType::SlideRight,
        Transition::None => gtk::StackTransitionType::None,
    }
}

fn set_markup_or_text(l: &gtk::Label, text: &str, markup: bool) {
    if markup && gtk::pango::parse_markup(text, '\0').is_ok() {
        l.set_markup(text);
    } else {
        l.set_text(text);
    }
}

fn load_texture(ctx: &Ctx, path: Option<&str>, data: Option<&str>) -> Option<gdk::Texture> {
    if let Some(d) = data {
        let bytes = glib::base64_decode(d);
        return gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)).ok();
    }
    let file = ctx.resolve_path(path?)?;
    gdk::Texture::from_filename(file).ok()
}

fn set_icon(img: &gtk::Image, ctx: &Ctx, name: &str) {
    if name.contains('/') {
        match ctx.resolve_path(name) {
            Some(p) => img.set_from_file(Some(p)),
            None => img.clear(),
        }
    } else {
        img.set_icon_name(Some(name));
    }
}

fn draw_graph(cr: &gtk::cairo::Context, w: f64, h: f64, values: &[f64], max: Option<f64>, style: GraphStyle, tint: Option<&str>) {
    if values.is_empty() || w <= 0.0 || h <= 0.0 {
        return;
    }
    let max = max.unwrap_or_else(|| values.iter().cloned().fold(0.0, f64::max)).max(f64::EPSILON);
    let c = tint.and_then(color::rgba).or_else(|| color::rgba("accent")).unwrap_or(gdk::RGBA::WHITE);
    let (r, g, b) = (c.red() as f64, c.green() as f64, c.blue() as f64);
    let n = values.len();
    match style {
        GraphStyle::Bars => {
            let bw = w / n as f64;
            cr.set_source_rgba(r, g, b, 0.9);
            for (i, v) in values.iter().enumerate() {
                let bh = (v / max).clamp(0.0, 1.0) * h;
                vector::rounded(cr, i as f64 * bw + bw * 0.15, h - bh, bw * 0.7, bh, 1.5);
                let _ = cr.fill();
            }
        }
        GraphStyle::Line => {
            let step = if n > 1 { w / (n - 1) as f64 } else { w };
            let y = |v: f64| h - 1.0 - (v / max).clamp(0.0, 1.0) * (h - 2.0);
            cr.move_to(0.0, y(values[0]));
            for (i, v) in values.iter().enumerate().skip(1) {
                cr.line_to(i as f64 * step, y(*v));
            }
            cr.set_source_rgba(r, g, b, 1.0);
            cr.set_line_width(1.5);
            cr.set_line_join(gtk::cairo::LineJoin::Round);
            let _ = cr.stroke_preserve();
            cr.line_to(w, h);
            cr.line_to(0.0, h);
            cr.close_path();
            cr.set_source_rgba(r, g, b, 0.18);
            let _ = cr.fill();
        }
    }
}

struct Builder<'a> {
    ctx: &'a Ctx,
    nodes: HashMap<String, (Handle, Vec<String>)>,
    states: Vec<StateProps>,
    counter: usize,
}

impl Builder<'_> {
    fn build(&mut self, node: &Node, parent: Option<Orientation>) -> gtk::Widget {
        let ctx = self.ctx;
        let handle = match &node.kind {
            NodeKind::Box { orientation, spacing, align: a, children } => {
                let o = match orientation {
                    Orientation::Horizontal => gtk::Orientation::Horizontal,
                    Orientation::Vertical => gtk::Orientation::Vertical,
                };
                let b = gtk::Box::new(o, (*spacing).clamp(0, 64));
                for c in children {
                    let w = self.build(c, Some(*orientation));
                    match orientation {
                        Orientation::Horizontal if c.height.is_none() => w.set_valign(align(*a)),
                        Orientation::Vertical if c.width.is_none() => w.set_halign(align(*a)),
                        _ => {}
                    }
                    b.append(&w);
                }
                Handle::Box(b)
            }
            NodeKind::Overlay { children } => {
                let o = gtk::Overlay::new();
                for (i, c) in children.iter().enumerate() {
                    let w = self.build(c, None);
                    if i == 0 {
                        o.set_child(Some(&w));
                    } else {
                        o.add_overlay(&w);
                        o.set_measure_overlay(&w, false);
                    }
                }
                Handle::Overlay(o)
            }
            NodeKind::Stack { show, transition: t, children } => {
                let s = gtk::Stack::new();
                s.set_transition_type(transition(*t));
                s.set_transition_duration(180);
                s.set_hhomogeneous(false);
                s.set_interpolate_size(true);
                for (i, c) in children.iter().enumerate() {
                    let w = self.build(c, None);
                    let name = c.id.clone().unwrap_or_else(|| i.to_string());
                    s.add_named(&w, Some(&name));
                }
                if let Some(v) = show {
                    s.set_visible_child_name(v);
                }
                Handle::Stack(s)
            }
            NodeKind::Label { text, markup } => {
                let l = gtk::Label::new(None);
                set_markup_or_text(&l, text, *markup);
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                l.set_single_line_mode(ctx.max_height.is_some());
                if ctx.max_height.is_none() {
                    l.set_wrap(true);
                    l.set_xalign(0.0);
                }
                Handle::Label(l)
            }
            NodeKind::Icon { name, size } => {
                let i = gtk::Image::new();
                set_icon(&i, ctx, name);
                let s = size.unwrap_or(16).clamp(8, ctx.max_height.unwrap_or(128));
                i.set_pixel_size(s);
                Handle::Icon(i)
            }
            NodeKind::Image { path, data, radius } => {
                let p = gtk::Picture::new();
                p.set_content_fit(gtk::ContentFit::Cover);
                p.set_can_shrink(true);
                p.set_overflow(gtk::Overflow::Hidden);
                if let Some(t) = load_texture(ctx, path.as_deref(), data.as_deref()) {
                    p.set_paintable(Some(&t));
                }
                if let Some(r) = radius {
                    let mut m = Map::new();
                    m.insert("radius".into(), (*r).into());
                    p.add_css_class(&style_class(&look_decls(&m, None)));
                }
                Handle::Image(p)
            }
            NodeKind::Badge { text } => {
                let l = gtk::Label::new(Some(text));
                l.add_css_class("plugin-badge");
                l.set_valign(gtk::Align::Center);
                Handle::Badge(l)
            }
            NodeKind::Progress { value } => {
                let b = gtk::LevelBar::for_interval(0.0, 1.0);
                b.set_value(value.clamp(0.0, 1.0));
                b.set_valign(gtk::Align::Center);
                b.set_size_request(32, 4);
                b.add_css_class("plugin-progress");
                Handle::Progress(b)
            }
            NodeKind::Graph { values, max, style } => {
                let a = gtk::DrawingArea::new();
                let data = Rc::new(RefCell::new((values.clone(), *max, *style, node.color.clone())));
                let d = data.clone();
                a.set_draw_func(move |_, cr, w, h| {
                    let d = d.borrow();
                    draw_graph(cr, w as f64, h as f64, &d.0, d.1, d.2, d.3.as_deref());
                });
                a.set_content_width(node.width.unwrap_or(40));
                a.set_content_height(node.height.unwrap_or(ctx.max_height.map_or(48, |h| h - 12)));
                Handle::Graph(a, data)
            }
            NodeKind::Spinner => {
                let s = gtk::Spinner::new();
                s.start();
                Handle::Spinner(s)
            }
            NodeKind::Vector(v) => {
                let a = gtk::DrawingArea::new();
                let data = Rc::new(RefCell::new(v.clone()));
                let d = data.clone();
                a.set_draw_func(move |_, cr, w, h| vector::draw(cr, &d.borrow(), w as f64, h as f64));
                a.set_content_width(node.width.unwrap_or(16));
                a.set_content_height(node.height.unwrap_or(16));
                Handle::Vector(a, data)
            }
            NodeKind::Frame => {
                let p = gtk::Picture::new();
                p.set_can_shrink(true);
                p.set_content_fit(gtk::ContentFit::Fill);
                Handle::Frame(p)
            }
            NodeKind::Row { icon, title, subtitle, action } => {
                let b = gtk::Button::new();
                b.add_css_class("flat");
                b.add_css_class("plugin-row");
                let h = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                let img = gtk::Image::new();
                img.set_pixel_size(16);
                match icon {
                    Some(i) => set_icon(&img, ctx, i),
                    None => img.set_visible(false),
                }
                let v = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let t = gtk::Label::new(Some(title));
                t.set_xalign(0.0);
                t.set_ellipsize(gtk::pango::EllipsizeMode::End);
                let s = gtk::Label::new(subtitle.as_deref());
                s.set_xalign(0.0);
                s.add_css_class("dim-label");
                s.add_css_class("caption");
                s.set_visible(subtitle.is_some());
                s.set_ellipsize(gtk::pango::EllipsizeMode::End);
                v.append(&t);
                v.append(&s);
                h.append(&img);
                h.append(&v);
                b.set_child(Some(&h));
                let control = action.clone().or_else(|| node.id.clone());
                let c = ctx.clone();
                b.connect_clicked(move |_| c.emit(EventKind::Action, control.clone(), None));
                Handle::Row(b, img, t, s)
            }
            NodeKind::Toggle { label, active } => {
                let h = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                let l = gtk::Label::new(label.as_deref());
                l.set_hexpand(true);
                l.set_xalign(0.0);
                l.set_visible(label.is_some());
                let s = gtk::Switch::new();
                s.set_active(*active);
                s.set_valign(gtk::Align::Center);
                let quiet = Rc::new(Cell::new(false));
                let (c, q, id) = (ctx.clone(), quiet.clone(), node.id.clone());
                s.connect_state_set(move |_, on| {
                    if !q.get() {
                        c.emit(EventKind::Toggle, id.clone(), Some(on.into()));
                    }
                    glib::Propagation::Proceed
                });
                h.append(&l);
                h.append(&s);
                Handle::Toggle(s, l, quiet)
            }
            NodeKind::Slider { min, max, step, value, icon } => {
                let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                if let Some(i) = icon {
                    let img = gtk::Image::new();
                    set_icon(&img, ctx, i);
                    h.append(&img);
                }
                let (lo, hi) = if max > min { (*min, *max) } else { (0.0, 100.0) };
                let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, lo, hi, step.max(f64::EPSILON));
                s.set_value(*value);
                s.set_hexpand(true);
                s.set_draw_value(false);
                let quiet = Rc::new(Cell::new(false));
                let pending: Rc<Cell<bool>> = Rc::default();
                let (c, q, id) = (ctx.clone(), quiet.clone(), node.id.clone());
                // At most every 50 ms while dragging, and the final value.
                s.connect_value_changed(move |s| {
                    if q.get() || pending.replace(true) {
                        return;
                    }
                    let (c, id, s, pending) = (c.clone(), id.clone(), s.clone(), pending.clone());
                    glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
                        pending.set(false);
                        c.emit(EventKind::Change, id, Some(s.value().into()));
                    });
                });
                h.append(&s);
                Handle::Slider(s, quiet)
            }
            NodeKind::Button { label, icon, action } => {
                let b = gtk::Button::new();
                match (label, icon) {
                    (Some(l), Some(i)) => {
                        let h = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                        let img = gtk::Image::new();
                        set_icon(&img, ctx, i);
                        h.append(&img);
                        h.append(&gtk::Label::new(Some(l)));
                        b.set_child(Some(&h));
                    }
                    (Some(l), None) => b.set_label(l),
                    (None, Some(i)) => {
                        let img = gtk::Image::new();
                        set_icon(&img, ctx, i);
                        b.set_child(Some(&img));
                        b.add_css_class("flat");
                    }
                    (None, None) => {}
                }
                let control = action.clone().or_else(|| node.id.clone());
                let c = ctx.clone();
                b.connect_clicked(move |_| c.emit(EventKind::Action, control.clone(), None));
                Handle::Button(b)
            }
            NodeKind::Entry { text, placeholder } => {
                let e = gtk::Entry::new();
                e.set_text(text);
                e.set_placeholder_text(placeholder.as_deref());
                let (c, id) = (ctx.clone(), node.id.clone());
                e.connect_activate(move |e| c.emit(EventKind::Submit, id.clone(), Some(e.text().to_string().into())));
                Handle::Entry(e)
            }
            NodeKind::Separator => Handle::Separator(gtk::Separator::new(gtk::Orientation::Horizontal)),
            NodeKind::Scroll { max_height, child } => {
                let s = gtk::ScrolledWindow::new();
                s.set_hscrollbar_policy(gtk::PolicyType::Never);
                s.set_propagate_natural_height(true);
                s.set_max_content_height(max_height.unwrap_or(400).clamp(40, 900));
                let w = self.build(child, None);
                s.set_child(Some(&w));
                Handle::Scroll(s)
            }
        };
        let w = handle.widget();
        if let Some([t, r, b, l]) = node.margin {
            w.set_margin_top(t.clamp(0, 64));
            w.set_margin_end(r.clamp(0, 64));
            w.set_margin_bottom(b.clamp(0, 64));
            w.set_margin_start(l.clamp(0, 64));
        }
        let cap = |v: i32| self.ctx.max_height.map_or(v, |h| v.min(h));
        if !matches!(node.kind, NodeKind::Graph { .. } | NodeKind::Vector(_)) {
            w.set_size_request(node.width.unwrap_or(-1).clamp(-1, 1600), node.height.map(cap).unwrap_or(-1).clamp(-1, 1600));
        } else if let Some(a) = w.downcast_ref::<gtk::DrawingArea>()
            && let Some(h) = node.height
        {
            a.set_content_height(cap(h));
        }
        if node.expand {
            match parent {
                Some(Orientation::Vertical) => w.set_vexpand(true),
                _ => w.set_hexpand(true),
            }
        }
        if let Some(t) = &node.tooltip {
            w.set_tooltip_text(Some(t));
        }
        if node.visible == Some(false) {
            w.set_visible(false);
        }
        for c in &node.class {
            if c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
                w.add_css_class(c);
            }
        }
        let classes: Rc<RefCell<Vec<String>>> = Rc::default();
        let props = node_props(node);
        set_look(&w, &classes, &props, self.ctx.max_height);
        if node.hover.is_some() || node.pressed.is_some() {
            let keys: HashSet<&String> = node.hover.iter().chain(node.pressed.iter()).flat_map(|m| m.keys()).collect();
            let original = keys.into_iter().map(|k| (k.clone(), props.get(k).cloned().unwrap_or(Value::Null))).collect();
            self.states.push(StateProps { handle: handle.clone(), classes: classes.clone(), hover: node.hover.clone(), pressed: node.pressed.clone(), original });
        }
        let id = node.id.clone().unwrap_or_else(|| {
            self.counter += 1;
            format!("#{}", self.counter)
        });
        self.nodes.insert(id, (handle, classes.borrow().clone()));
        // Keep the shared look classes reachable for patches.
        unsafe { w.set_data("nv-classes", classes) };
        w
    }
}

fn classes_of(w: &gtk::Widget) -> Rc<RefCell<Vec<String>>> {
    unsafe { w.data::<Rc<RefCell<Vec<String>>>>("nv-classes").map(|p| p.as_ref().clone()).unwrap_or_default() }
}

/// Render a tree.
pub fn render(node: &Node, ctx: &Ctx) -> Rendered {
    let mut b = Builder { ctx, nodes: HashMap::new(), states: vec![], counter: 0 };
    let root = b.build(node, None);
    root.add_css_class(&css::plugin_class(&ctx.plugin));
    Rendered { root, nodes: b.nodes, states: b.states }
}

/// The standard row for an item without a view: icon, text, badge, progress.
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

impl Rendered {
    /// Apply one node's changes. Unknown ids and properties are ignored.
    pub fn patch(&self, p: &NodePatch, ctx: &Ctx) {
        let Some((h, _)) = self.nodes.get(&p.id) else { return };
        apply(h, &p.set, ctx);
    }

    /// The picture a `frame` node shows.
    pub fn frame(&self, node: &str) -> Option<gtk::Picture> {
        match self.nodes.get(node) {
            Some((Handle::Frame(p), _)) => Some(p.clone()),
            _ => None,
        }
    }

    /// Ids of the frame nodes.
    pub fn frame_nodes(&self) -> Vec<String> {
        self.nodes.iter().filter(|(_, (h, _))| matches!(h, Handle::Frame(_))).map(|(k, _)| k.clone()).collect()
    }

    /// Hover and pressed states: wire them to the widget that receives the pointer.
    pub fn attach_states(self: &Rc<Self>, target: &gtk::Widget, ctx: &Ctx) {
        if self.states.is_empty() {
            return;
        }
        let motion = gtk::EventControllerMotion::new();
        let (r, c) = (Rc::downgrade(self), ctx.clone());
        motion.connect_enter(move |_, _, _| {
            if let Some(r) = r.upgrade() {
                r.state(&c, true, false);
            }
        });
        let (r, c) = (Rc::downgrade(self), ctx.clone());
        motion.connect_leave(move |_| {
            if let Some(r) = r.upgrade() {
                r.state(&c, false, false);
            }
        });
        target.add_controller(motion);
        let click = gtk::GestureClick::new();
        click.set_button(0);
        let (r, c) = (Rc::downgrade(self), ctx.clone());
        click.connect_pressed(move |_, _, _, _| {
            if let Some(r) = r.upgrade() {
                r.state(&c, true, true);
            }
        });
        let (r, c) = (Rc::downgrade(self), ctx.clone());
        click.connect_released(move |_, _, _, _| {
            if let Some(r) = r.upgrade() {
                r.state(&c, true, false);
            }
        });
        target.add_controller(click);
    }

    fn state(&self, ctx: &Ctx, hover: bool, pressed: bool) {
        for s in &self.states {
            let mut props = s.original.clone();
            if hover && let Some(h) = &s.hover {
                props.extend(h.clone());
            }
            if pressed && let Some(p) = &s.pressed {
                props.extend(p.clone());
            }
            apply(&s.handle, &props, ctx);
            let _ = &s.classes;
        }
    }
}

fn f(v: &Value) -> Option<f64> {
    v.as_f64()
}

fn apply(h: &Handle, set: &Map<String, Value>, ctx: &Ctx) {
    let w = h.widget();
    for (k, v) in set {
        match (k.as_str(), h) {
            ("visible", _) if v.is_boolean() => w.set_visible(v.as_bool().unwrap_or(true)),
            ("tooltip", _) => w.set_tooltip_text(v.as_str()),
            ("text", Handle::Label(l)) => {
                let markup = set.get("markup").and_then(Value::as_bool).unwrap_or(l.uses_markup());
                set_markup_or_text(l, v.as_str().unwrap_or(""), markup);
            }
            ("text", Handle::Badge(l)) => l.set_text(v.as_str().unwrap_or("")),
            ("text", Handle::Entry(e)) => e.set_text(v.as_str().unwrap_or("")),
            ("name" | "icon", Handle::Icon(i)) => set_icon(i, ctx, v.as_str().unwrap_or("")),
            ("icon", Handle::Row(_, i, _, _)) => {
                i.set_visible(v.is_string());
                set_icon(i, ctx, v.as_str().unwrap_or(""));
            }
            ("title", Handle::Row(_, _, t, _)) => t.set_text(v.as_str().unwrap_or("")),
            ("subtitle", Handle::Row(_, _, _, s)) => {
                s.set_visible(v.is_string());
                s.set_text(v.as_str().unwrap_or(""));
            }
            ("label", Handle::Toggle(_, l, _)) => {
                l.set_visible(v.is_string());
                l.set_text(v.as_str().unwrap_or(""));
            }
            ("label", Handle::Button(b)) => b.set_label(v.as_str().unwrap_or("")),
            ("active", Handle::Toggle(s, _, q)) => {
                q.set(true);
                s.set_active(v.as_bool().unwrap_or(false));
                s.set_state(v.as_bool().unwrap_or(false));
                q.set(false);
            }
            ("value", Handle::Slider(s, q)) => {
                q.set(true);
                s.set_value(f(v).unwrap_or(0.0));
                q.set(false);
            }
            ("value", Handle::Progress(b)) => b.set_value(f(v).unwrap_or(0.0).clamp(0.0, 1.0)),
            ("values", Handle::Graph(a, d)) => {
                d.borrow_mut().0 = v.as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
                a.queue_draw();
            }
            ("max", Handle::Graph(a, d)) => {
                d.borrow_mut().1 = f(v);
                a.queue_draw();
            }
            ("svg", Handle::Vector(a, d)) => {
                d.borrow_mut().svg = v.as_str().map(String::from);
                a.queue_draw();
            }
            ("commands", Handle::Vector(a, d)) => {
                if let Ok(c) = serde_json::from_value(v.clone()) {
                    let mut d = d.borrow_mut();
                    d.commands = c;
                    d.svg = None;
                }
                a.queue_draw();
            }
            ("show", Handle::Stack(s)) => {
                if let Some(name) = v.as_str()
                    && s.child_by_name(name).is_some()
                {
                    s.set_visible_child_name(name);
                }
            }
            ("path" | "data", Handle::Image(p)) => {
                let (path, data) = if k == "path" { (v.as_str(), None) } else { (None, v.as_str()) };
                p.set_paintable(load_texture(ctx, path, data).as_ref());
            }
            ("spinning", Handle::Spinner(s)) => s.set_spinning(v.as_bool().unwrap_or(true)),
            ("class", _) => {
                // Replace the plugin's own classes (not the generated ones).
                for c in w.css_classes() {
                    let c = c.to_string();
                    if !c.starts_with("nv-") && !c.starts_with("plugin-") && c != "flat" {
                        w.remove_css_class(&c);
                    }
                }
                for c in v.as_array().into_iter().flatten().filter_map(Value::as_str) {
                    if c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
                        w.add_css_class(c);
                    }
                }
            }
            _ => {}
        }
    }
    if ["color", "background", "font_size", "radius"].iter().any(|k| set.contains_key(*k)) {
        set_look(&w, &classes_of(&w), set, ctx.max_height);
    }
}

/// A menu from a plugin's entries. Activating an entry calls `on_action` with its id.
pub fn menu(entries: &[MenuEntry], on_action: Rc<dyn Fn(String)>) -> (gio::Menu, gio::SimpleActionGroup) {
    let group = gio::SimpleActionGroup::new();
    let mut n = 0usize;
    let root = build_menu(entries, &group, &on_action, &mut n);
    (root, group)
}

fn build_menu(entries: &[MenuEntry], group: &gio::SimpleActionGroup, on: &Rc<dyn Fn(String)>, n: &mut usize) -> gio::Menu {
    let root = gio::Menu::new();
    let mut section = gio::Menu::new();
    for e in entries.iter().take(200) {
        if e.separator {
            if section.n_items() > 0 {
                root.append_section(None, &section);
            }
            section = gio::Menu::new();
            continue;
        }
        if !e.children.is_empty() {
            let sub = build_menu(&e.children, group, on, n);
            section.append_submenu(Some(&e.label), &sub);
            continue;
        }
        *n += 1;
        let name = format!("e{n}");
        let action = match e.checked {
            Some(c) => gio::SimpleAction::new_stateful(&name, None, &c.to_variant()),
            None => gio::SimpleAction::new(&name, None),
        };
        action.set_enabled(e.enabled);
        let (on, id) = (on.clone(), e.id.clone());
        action.connect_activate(move |_, _| on(id.clone()));
        group.add_action(&action);
        let item = gio::MenuItem::new(Some(&e.label), Some(&format!("pm.{name}")));
        if let Some(i) = &e.icon
            && !i.contains('/')
        {
            item.set_icon(&gio::ThemedIcon::new(i));
        }
        section.append_item(&item);
    }
    if section.n_items() > 0 {
        root.append_section(None, &section);
    }
    root
}

/// Show one shared-memory frame (ARGB8888 premultiplied, little-endian: B G R A).
pub fn show_frame(picture: &gtk::Picture, bytes: &[u8], width: i32, height: i32, stride: i32) {
    let tex = gdk::MemoryTexture::new(width, height, gdk::MemoryFormat::B8g8r8a8Premultiplied, &glib::Bytes::from(bytes), stride as usize);
    picture.set_paintable(Some(&tex));
}
