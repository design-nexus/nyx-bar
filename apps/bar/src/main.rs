//! Nyx Bar: a full-width top bar on every monitor. The bar itself only hosts: every
//! item, the workspaces and the clock included, comes from a plugin (see
//! `docs/plugins.md`). `nyx plugin` adds, removes and arranges them.
//!
//! `--only <connector>` shows the bar on one output only, as a separate test copy.

mod bar;
mod dbus;
mod theme;

use gtk::prelude::*;

pub const APP_ID: &str = "io.github.design_nexus.nyx.Bar";

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
            "--version" => {
                println!("nyx-bar {}", env!("CARGO_PKG_VERSION"));
                return std::process::ExitCode::SUCCESS;
            }
            "-h" | "--help" => {
                println!(
                    "Usage: nyx-bar [--only <output>]\n\n\
                     Runs the bar (normally as the nyx-bar user service).\n\
                     --only   show the bar on this output only (e.g. HEADLESS-1), as a test copy\n\n\
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
    app.connect_startup(|_| theme::install());
    let started = std::cell::Cell::new(false);
    app.connect_activate(move |app| {
        if !started.replace(true) {
            bar::start(app, only.clone());
        }
    });
    let code = app.run_with_args::<&str>(&[]);
    std::process::ExitCode::from(code.get())
}
