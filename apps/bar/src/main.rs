//! Nyx Bar: a full-width top bar on every monitor. The bar itself only hosts: every
//! item, the workspaces and the clock included, comes from a plugin (see
//! `docs/plugins.md`). `nyx plugin` adds, removes and arranges them.
//!
//! `--only <connector>` shows the bar on one output only, as a separate test copy.
//! `--ui qml|gtk` picks the Quickshell interface (`nyx-bar-backend` and the bar's QML in
//! the shared `nyx-shell-qs` runtime) or this GTK one.

mod bar;
mod dbus;
mod theme;

use gtk::prelude::*;
use nyx_ui_link::switch::{self, Component, Ui};

pub const APP_ID: &str = "io.github.design_nexus.nyx.Bar";

const COMPONENT: Component = Component { app: "nyx-bar", gtk_unit: "nyx-bar.service", backend_units: &["nyx-bar-backend.service"] };

/// The Quickshell bar is on: linked by `--ui qml`, or installed with the system (NyxOS).
fn qml_is_on() -> bool {
    if switch::current(&COMPONENT) == Ui::Qml {
        return true;
    }
    switch::managed_by_system()
        && std::env::var("XDG_DATA_DIRS")
            .unwrap_or_default()
            .split(':')
            .chain(["/run/current-system/sw/share"])
            .filter(|d| !d.is_empty())
            .any(|d| std::path::Path::new(d).join("nyx-shell/components").join(COMPONENT.app).join("Main.qml").is_file())
}

fn wants_gpu() -> bool {
    std::fs::read_to_string(nyx_bar_core::Config::path())
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("renderer").and_then(|r| r.as_str()).map(|r| r == "gpu"))
        .unwrap_or(false)
}

fn main() -> std::process::ExitCode {
    let mut only = std::env::var("NYX_BAR_ONLY").ok().filter(|s| !s.is_empty());
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--only" => only = args.next(),
            "--focus" => {
                return match switch::forward("nyx-bar", "Focus", serde_json::json!({})) {
                    Some(Ok(_)) => std::process::ExitCode::SUCCESS,
                    Some(Err(e)) => {
                        eprintln!("nyx-bar: {e}");
                        std::process::ExitCode::FAILURE
                    }
                    None => {
                        eprintln!("nyx-bar: the bar isn't running");
                        std::process::ExitCode::FAILURE
                    }
                };
            }
            "--layout-json" => {
                let plugins = nyx_plugin::discovery::discover().plugins;
                let json = nyx_bar_core::layout_json(&nyx_config::Layers::standard(), &plugins, &nyx_plugin::State::load());
                println!("{json}");
                return std::process::ExitCode::SUCCESS;
            }
            "--ui" => {
                return match args.next().as_deref().map(Ui::parse) {
                    Some(Some(ui)) => match switch::set(&COMPONENT, ui) {
                        Ok(done) => {
                            println!("{done}");
                            std::process::ExitCode::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("nyx-bar: {e}");
                            std::process::ExitCode::FAILURE
                        }
                    },
                    Some(None) => {
                        eprintln!("nyx-bar: --ui takes qml or gtk");
                        std::process::ExitCode::from(2)
                    }
                    None => {
                        println!("{}", if qml_is_on() { "qml" } else { "gtk" });
                        std::process::ExitCode::SUCCESS
                    }
                };
            }
            "--version" => {
                println!("nyx-bar {}", env!("CARGO_PKG_VERSION"));
                return std::process::ExitCode::SUCCESS;
            }
            "-h" | "--help" => {
                println!(
                    "Usage: nyx-bar [--only <output>] [--layout-json] [--ui [qml|gtk]] [--focus]\n\n\
                     Runs the bar (normally as the nyx-bar user service).\n\
                     --only          show the bar on this output only (e.g. HEADLESS-1), as a test copy\n\
                     --layout-json   print the layout in use, its entries' names and what it doesn't place\n\
                     --ui            use the Quickshell (qml) or the GTK interface; alone, show which\n\
                     --focus         give the running bar the keyboard (Quickshell bar)\n\n\
                     Plugins: `nyx plugin list`, `nyx plugin add`, `nyx plugin configure`.\n\
                     Layout: ~/.config/nyx/bar.toml. Style: ~/.config/nyx/bar.css."
                );
                return std::process::ExitCode::SUCCESS;
            }
            other => {
                eprintln!("nyx-bar: unknown option {other}");
                return std::process::ExitCode::from(2);
            }
        }
    }
    if only.is_none() && qml_is_on() {
        eprintln!("nyx-bar: the Quickshell bar is on (nyx-bar-backend and nyx-shell-qs); switch back with: nyx-bar --ui gtk");
        return std::process::ExitCode::SUCCESS;
    }
    // A bar redraws little: GTK's cairo renderer draws it as well for about 70 MB less
    // than the GPU renderers, whatever the session asks for. `renderer = "gpu"` in
    // bar.toml leaves the choice to GTK and GSK_RENDERER.
    if !wants_gpu() {
        unsafe { std::env::set_var("GSK_RENDERER", "cairo") };
    }
    let (id, flags) = match &only {
        Some(_) => (format!("{APP_ID}.Test"), gtk::gio::ApplicationFlags::NON_UNIQUE),
        None => (APP_ID.to_string(), Default::default()),
    };
    let app = gtk::Application::builder().application_id(id).flags(flags).build();
    let test = only.is_some();
    app.connect_startup(move |_| theme::install(test));
    let started = std::cell::Cell::new(false);
    app.connect_activate(move |app| {
        if !started.replace(true) {
            bar::start(app, only.clone());
        }
    });
    let code = app.run_with_args::<&str>(&[]);
    std::process::ExitCode::from(code.get())
}
