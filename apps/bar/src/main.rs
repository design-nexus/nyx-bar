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
