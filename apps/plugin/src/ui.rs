//! Prompts: `gum` when it's installed, plain questions otherwise. Without a terminal,
//! anything that needs confirming fails unless `--yes` was given.

use anyhow::{Result, bail};
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};

pub fn interactive() -> bool {
    unsafe { libc::isatty(0) == 1 && libc::isatty(1) == 1 }
}

fn gum() -> bool {
    std::env::var_os("NYX_PLUGIN_NO_GUM").is_none() && nyx_plugin::discovery::on_path("gum")
}

fn ask(prompt: &str) -> String {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    line.trim().to_string()
}

/// Yes or no. `yes` (from `--yes`) answers yes without asking.
pub fn confirm(prompt: &str, yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if !interactive() {
        bail!("{prompt} — refusing without a terminal; pass --yes");
    }
    if gum() {
        return Ok(Command::new("gum").args(["confirm", prompt]).status().is_ok_and(|s| s.success()));
    }
    let a = ask(&format!("{prompt} [y/N] "));
    Ok(matches!(a.to_lowercase().as_str(), "y" | "yes"))
}

/// Like `confirm`, for something risky: the answer starts at no, and `--yes` alone
/// isn't enough (`allow` is the extra flag that says so).
pub fn confirm_risky(prompt: &str, yes: bool, allow: bool, flag: &str) -> Result<bool> {
    if yes && allow {
        return Ok(true);
    }
    if !interactive() {
        bail!("{prompt} — refusing without a terminal; pass --yes {flag}");
    }
    if gum() {
        return Ok(Command::new("gum").args(["confirm", "--default=false", prompt]).status().is_ok_and(|s| s.success()));
    }
    let a = ask(&format!("{prompt} [y/N] "));
    Ok(matches!(a.to_lowercase().as_str(), "y" | "yes"))
}

/// Pick any number of options; `selected` start ticked. Returns their indexes.
pub fn choose_many(header: &str, options: &[String], selected: &[bool]) -> Result<Vec<usize>> {
    if !interactive() {
        bail!("choosing needs a terminal; use the --bar, --control-center, --desktop, --all or --none flags");
    }
    if gum() {
        let picked: Vec<&str> = options.iter().zip(selected).filter(|(_, s)| **s).map(|(o, _)| o.as_str()).collect();
        let mut cmd = Command::new("gum");
        cmd.args(["choose", "--no-limit", "--header", header]);
        if !picked.is_empty() {
            cmd.arg(format!("--selected={}", picked.join(",")));
        }
        cmd.args(options).stdout(Stdio::piped());
        let out = cmd.output()?;
        if !out.status.success() {
            bail!("cancelled");
        }
        let chosen: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(String::from).collect();
        return Ok(options.iter().enumerate().filter(|(_, o)| chosen.contains(o)).map(|(i, _)| i).collect());
    }
    let mut on: Vec<bool> = selected.to_vec();
    loop {
        println!("\n{header}");
        for (i, o) in options.iter().enumerate() {
            println!("  {} [{}] {o}", i + 1, if on[i] { "x" } else { " " });
        }
        let a = ask("Numbers to switch on or off (Enter when done): ");
        if a.is_empty() {
            break;
        }
        for n in a.split([' ', ',']).filter_map(|s| s.trim().parse::<usize>().ok()) {
            if (1..=on.len()).contains(&n) {
                on[n - 1] = !on[n - 1];
            }
        }
    }
    Ok((0..on.len()).filter(|i| on[*i]).collect())
}

/// Pick one; `default` is pre-selected.
pub fn choose_one(header: &str, options: &[&str], default: usize) -> Result<usize> {
    if !interactive() {
        return Ok(default);
    }
    if gum() {
        let out = Command::new("gum")
            .args(["choose", "--header", header, &format!("--selected={}", options[default])])
            .args(options)
            .stdout(Stdio::piped())
            .output()?;
        if !out.status.success() {
            bail!("cancelled");
        }
        let c = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return Ok(options.iter().position(|o| *o == c).unwrap_or(default));
    }
    println!("\n{header}");
    for (i, o) in options.iter().enumerate() {
        println!("  {} {o}{}", i + 1, if i == default { " (default)" } else { "" });
    }
    let a = ask("Choose (Enter for the default): ");
    Ok(a.parse::<usize>().ok().filter(|n| (1..=options.len()).contains(n)).map(|n| n - 1).unwrap_or(default))
}

pub fn warn(text: &str) {
    eprintln!("\n\x1b[33m{text}\x1b[0m\n");
}

/// Show long text through `delta` or `less` when there's a terminal.
pub fn page(text: &str) {
    if text.trim().is_empty() {
        return;
    }
    if interactive() {
        for (prog, args) in [("delta", vec!["--paging=never"]), ("less", vec!["-R", "-F", "-X"])] {
            if nyx_plugin::discovery::on_path(prog)
                && let Ok(mut c) = Command::new(prog).args(args).stdin(Stdio::piped()).spawn()
            {
                if let Some(mut i) = c.stdin.take() {
                    let _ = i.write_all(text.as_bytes());
                }
                let _ = c.wait();
                return;
            }
        }
    }
    println!("{text}");
}

/// A warning that matters more: in red.
pub fn danger(text: &str) {
    eprintln!("\x1b[1;31m{text}\x1b[0m\n");
}
