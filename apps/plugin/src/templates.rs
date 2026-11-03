//! `nyx plugin new`: a working plugin to start from.

use anyhow::{Result, bail};
use std::path::Path;

fn manifest(id: &str, name: &str, exec: &str) -> String {
    format!(
        r#"id = "{id}"
name = "{name}"
version = "0.1.0"
author = ""
license = "MIT"
description = "What it shows."
api_version = 1
kinds = ["bar"]
exec = {exec}
needs = []

# What it may do once installed from elsewhere (third-party plugins run sandboxed):
[permissions]
network = false
# session_talk = ["org.mpris.MediaPlayer2.*"]
# host_commands = [["nyx-weather", "--toggle"]]

[bar]
mode = "stream"
section = "right"

[[bar.items]]
id = "main"
name = "{name}"
default = true

[settings]
greeting = "Hello"
"#
    )
}

const SH: &str = r#"#!/bin/sh
# A Nyx Bar plugin. Read JSON lines on stdin, write JSON lines on stdout.
# Reference: https://github.com/design-nexus/nyx-bar/blob/main/docs/plugins.md
read -r hello
greeting=$(printf '%s' "$hello" | sed -n 's/.*"greeting":"\([^"]*\)".*/\1/p')
count=0
show() { printf '{"type":"item","id":"main","icon":"face-smile-symbolic","text":"%s"}\n' "$1"; }
show "${greeting:-Hello}"
while read -r line; do
  case $line in
    *'"type":"shutdown"'*) exit 0 ;;
    *'"kind":"click"'*) count=$((count + 1)); show "${greeting:-Hello} ×$count" ;;
  esac
done
"#;

const PY: &str = r#"#!/usr/bin/env python3
"""A Nyx Bar plugin in Python. `nyx_plugin.py` (in this folder) does the protocol."""
import nyx_plugin as nyx

plugin = nyx.Plugin()
count = 0

def show():
    text = plugin.settings.get("greeting", "Hello") + (f" ×{count}" if count else "")
    plugin.item("main", view=nyx.hbox([nyx.icon("face-smile-symbolic"), nyx.label(text)]))

show()
for event in plugin.events():
    if event.get("kind") == "click":
        count += 1
        show()
"#;

const RS: &str = r#"use nyx_plugin_sdk::protocol::EventKind;
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin};

fn main() {
    let (plugin, inputs) = Plugin::<()>::start();
    let greeting = plugin.setting_str("greeting", "Hello");
    let mut count = 0;
    plugin.item(item("main").view(hbox([icon("face-smile-symbolic"), label(&greeting).id("text")])));
    for input in inputs {
        match input {
            Input::Event(e) if e.kind == EventKind::Click => {
                count += 1;
                plugin.patch("main", "text", [("text", format!("{greeting} ×{count}").into())]);
            }
            Input::Shutdown => break,
            _ => {}
        }
    }
}
"#;

pub fn create(dir: &Path, id: &str, kind: &str) -> Result<()> {
    if dir.exists() && std::fs::read_dir(dir)?.next().is_some() {
        bail!("{} isn't empty", dir.display());
    }
    std::fs::create_dir_all(dir)?;
    let short = id.rsplit('.').next().unwrap_or(id);
    let name = {
        let mut c = short.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    use std::os::unix::fs::PermissionsExt;
    let exec_file = |name: &str, text: &str| -> Result<()> {
        let p = dir.join(name);
        std::fs::write(&p, text)?;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))?;
        Ok(())
    };
    match kind {
        "sh" => {
            std::fs::write(dir.join("plugin.toml"), manifest(id, &name, "[\"sh\", \"plugin.sh\"]").replace("needs = []", "needs = [\"sh\"]"))?;
            exec_file("plugin.sh", SH)?;
        }
        "py" => {
            std::fs::write(dir.join("plugin.toml"), manifest(id, &name, "[\"python3\", \"plugin.py\"]").replace("needs = []", "needs = [\"python3\"]"))?;
            exec_file("plugin.py", PY)?;
            std::fs::write(dir.join("nyx_plugin.py"), include_str!("../../../sdk/python/nyx_plugin.py"))?;
        }
        "rust" => {
            let bin = short.replace('.', "-");
            std::fs::write(dir.join("plugin.toml"), manifest(id, &name, &format!("[\"{bin}\"]")))?;
            std::fs::create_dir_all(dir.join("src"))?;
            std::fs::write(dir.join("src/main.rs"), RS)?;
            std::fs::write(
                dir.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{bin}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nnyx-plugin-sdk = {{ git = \"https://github.com/design-nexus/nyx-bar.git\" }}\n\n[profile.release]\nlto = \"thin\"\nstrip = true\n"
                ),
            )?;
            std::fs::write(dir.join(".gitignore"), format!("target/\n{bin}\n*.tar.gz\n"))?;
            std::fs::write(
                dir.join("README.md"),
                format!(
                    "# {name}\n\nBuild it and put the program beside `plugin.toml`:\n\n```sh\ncargo build --release && cp target/release/{bin} .\nnyx plugin link .\n```\n\nTo publish, `nyx plugin pack .` makes the archive for `[artifacts]` and prints its hash.\n"
                ),
            )?;
        }
        _ => bail!("--sh, --py or --rust"),
    }
    Ok(())
}
