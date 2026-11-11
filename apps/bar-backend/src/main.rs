//! `nyx-bar-backend`: Nyx Bar without GTK, for its Quickshell interface (`qml/Bar` in this
//! repository), on NyxOS.
//!
//! Plugins and the layout are kept here as the GTK bar keeps them (`host.rs`); the
//! interface draws a bar per monitor and talks to this over
//! `$XDG_RUNTIME_DIR/nyx-bar/ui.sock` (nyx-ui-link):
//!
//! - topic `bar`: `{size, height, monitors: {name: {name, width, hidden, sections,
//!   items}}, styles, user}`; each section is a list of slots (`item`, `group`,
//!   `divider`), each item (by "plugin\titem") its view with paths resolved, `rev` (up
//!   when the plugin sends a new view), tooltip, classes, popover and menu; `styles` holds
//!   each plugin's style.css as rules (`css.rs`), `user` the user's bar.css;
//! - events `NodePatch` (`{plugin, item, monitor, nodes}`), and `TestClick` and `TestClose`
//!   for screenshots;
//! - operations: `Click` (`{monitor, plugin, item, button, anchor}`, answering what it
//!   did: `command`, `popover`, `menu` or `event`), `Scroll`, `ViewEvent`, `MenuAction`,
//!   `PopoverOpened`, `PopoverClosed`.
//!
//! It owns `io.github.design_nexus.nyx.Bar` and serves `Bar1`, as the GTK bar does.
//! `NYX_BAR_ONLY=<output>` with `NYX_BAR_UI_SOCKET` makes a test copy for one output (bus
//! name `…Bar.Test`).

mod css;
mod dbus;
mod host;
mod mono;

use nyx_plugin_protocol::EventKind;
use serde_json::{Value, json};

pub const APP_ID: &str = "io.github.design_nexus.nyx.Bar";

fn handle(s: &std::rc::Rc<host::Shared>, req: nyx_ui_link::Request) {
    let b = req.body.clone();
    let str_of = |k: &str| b.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let (monitor, plugin, item) = (str_of("monitor"), str_of("plugin"), str_of("item"));
    let reply = match req.op.as_str() {
        "Click" => {
            let button = b.get("button").and_then(Value::as_u64).unwrap_or(1) as u32;
            json!({"did": host::click(s, &plugin, &item, &monitor, button, host::anchor_of(&monitor, &b))})
        }
        "Scroll" => {
            host::scroll(s, &plugin, &item, &monitor, b.get("up").and_then(Value::as_bool).unwrap_or(false));
            json!({})
        }
        "ViewEvent" => {
            let kind = match str_of("kind").as_str() {
                "toggle" => EventKind::Toggle,
                "change" => EventKind::Change,
                "submit" => EventKind::Submit,
                _ => EventKind::Action,
            };
            let control = b.get("control").and_then(Value::as_str).map(String::from);
            host::send_event(s, &plugin, &item, &monitor, kind, None, None, control, b.get("value").cloned(), None);
            json!({})
        }
        "MenuAction" => {
            host::send_event(s, &plugin, &item, &monitor, EventKind::Action, None, None, Some(str_of("id")), None, None);
            json!({})
        }
        "PopoverOpened" => {
            host::send_event(s, &plugin, &item, &monitor, EventKind::PopoverOpen, Some(1), None, None, None, host::anchor_of(&monitor, &b));
            json!({})
        }
        "PopoverClosed" => {
            host::send_event(s, &plugin, &item, &monitor, EventKind::PopoverClose, None, None, None, None, None);
            json!({})
        }
        _ => {
            let op = req.op.clone();
            return req.err("unknown_op", op);
        }
    };
    req.ok(reply);
}

fn main() -> std::process::ExitCode {
    let mut only = std::env::var("NYX_BAR_ONLY").ok().filter(|s| !s.is_empty());
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--only" => only = args.next(),
            "-h" | "--help" => {
                println!(
                    "Usage: nyx-bar-backend [--only <output>]\n\n\
                     Nyx Bar's plugins and layout for its Quickshell interface (NyxOS).\n\
                     --only   the bar on this output only (e.g. HEADLESS-1), as a test copy"
                );
                return std::process::ExitCode::SUCCESS;
            }
            other => {
                eprintln!("nyx-bar-backend: unknown option {other}");
                return std::process::ExitCode::from(2);
            }
        }
    }
    let ctx = glib::MainContext::default();
    let _guard = ctx.acquire().expect("main context");
    let (req_tx, req_rx) = async_channel::unbounded::<nyx_ui_link::Request>();
    let link = match nyx_ui_link::serve(nyx_ui_link::Options::new("nyx-bar"), move |r| {
        let _ = req_tx.send_blocking(r);
    }) {
        Ok(l) => l,
        Err(nyx_ui_link::Error::AlreadyRunning(p)) => {
            eprintln!("nyx-bar-backend: already running at {}", p.display());
            return std::process::ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("nyx-bar-backend: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let s = host::start(link, only);

    match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
        Ok(conn) => {
            if let Err(e) = dbus::serve(&conn, &s) {
                eprintln!("nyx-bar-backend: couldn't serve Bar1: {e}");
            }
            let name = if s.test() { format!("{APP_ID}.Test") } else { APP_ID.to_string() };
            gio::bus_own_name_on_connection(
                &conn,
                &name,
                gio::BusNameOwnerFlags::REPLACE | gio::BusNameOwnerFlags::ALLOW_REPLACEMENT,
                |_, _| {},
                |_, name| eprintln!("nyx-bar-backend: {name} is taken (is the GTK bar running?)"),
            );
            std::mem::forget(conn);
        }
        Err(e) => eprintln!("nyx-bar-backend: no session bus: {e}"),
    }

    let s2 = s.clone();
    ctx.spawn_local(async move {
        while let Ok(req) = req_rx.recv().await {
            handle(&s2, req);
        }
    });
    glib::MainLoop::new(Some(&ctx), false).run();
    std::process::ExitCode::SUCCESS
}
