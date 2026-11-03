//! `nyx plugin`: add, update, remove, enable and arrange Nyx plugins (for the bar now,
//! the Control Center and desktop widgets later). Both `nyx` commands (Omarchy's and
//! NyxOS's) pass `nyx plugin …` here.

mod templates;
mod ui;

use anyhow::{Context, Result, bail};
use nyx_plugin::discovery::{self, Found, Plugin, Trust};
use nyx_plugin::index::{self, Entry};
use nyx_plugin::install::{self, Removed, Source, Staged};
use nyx_plugin::layout::{self, Position};
use nyx_plugin::manifest::{Kind, Manifest, WHOLE};
use nyx_plugin::state::{Editor, State, now_utc};
use nyx_plugin::{paths, permissions};
use std::path::{Path, PathBuf};

const USAGE: &str = "\
nyx plugin list [--json] [--kind K]     installed plugins, their parts, and what they use
nyx plugin search [TERM] [--refresh]    plugins in the Nyx plugin index
nyx plugin info ID|NAME|URL             what a plugin does and may do
nyx plugin add URL|NAME [options]       install from git or the index
    --ref TAG          a branch, tag or commit (URLs only)
    --enable           turn on its default parts      --all / --none
    --bar[=ITEM,…]     --control-center[=ITEM,…]      --desktop[=ITEM,…]
    --section S        where bar items go (left, center, right)
    --yes              don't ask (approves its permissions)
nyx plugin configure [ID]               choose its parts and where they go
nyx plugin update [ID] [--yes]          update git plugins (all, or one)
nyx plugin remove ID [--yes]            uninstall (built-in plugins can only be turned off)
nyx plugin enable ID[/ITEM] [--kind K] [--section S] [--before E | --after E | --index N] [--monitor M]
nyx plugin disable ID[/ITEM] [--kind K]
nyx plugin clone ID [NEW_ID]            copy a built-in plugin to change it
nyx plugin new ID [--sh | --py | --rust] [DIR]   start a plugin
nyx plugin link PATH                    use a working folder as a plugin
nyx plugin check PATH                   validate a plugin and try it in the sandbox
nyx plugin pack PATH [--out FILE]       make an [artifacts] archive and print its hash

Kinds: bar, control-center, desktop. Plugins live in ~/.local/share/nyx/plugins;
choices in ~/.config/nyx/plugins.toml; the bar's layout in ~/.config/nyx/bar.toml.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("nyx plugin: {e:#}");
        std::process::exit(1);
    }
}

/// Flags and positional arguments.
struct Args {
    pos: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

impl Args {
    fn parse(raw: &[String]) -> Args {
        let takes = ["--ref", "--section", "--before", "--after", "--index", "--monitor", "--kind", "--out"];
        let mut pos = vec![];
        let mut flags = vec![];
        let mut i = 0;
        while i < raw.len() {
            let a = &raw[i];
            if let Some(f) = a.strip_prefix("--") {
                match f.split_once('=') {
                    Some((k, v)) => flags.push((format!("--{k}"), Some(v.to_string()))),
                    None if takes.contains(&a.as_str()) => {
                        flags.push((a.clone(), raw.get(i + 1).cloned()));
                        i += 1;
                    }
                    None => flags.push((a.clone(), None)),
                }
            } else if a == "-y" {
                flags.push(("--yes".into(), None));
            } else {
                pos.push(a.clone());
            }
            i += 1;
        }
        Args { pos, flags }
    }

    fn has(&self, f: &str) -> bool {
        self.flags.iter().any(|(k, _)| k == f)
    }

    fn get(&self, f: &str) -> Option<&str> {
        self.flags.iter().find(|(k, _)| k == f).and_then(|(_, v)| v.as_deref())
    }

    fn yes(&self) -> bool {
        self.has("--yes")
    }
}

fn run(raw: &[String]) -> Result<()> {
    let Some(cmd) = raw.first() else {
        println!("{USAGE}");
        return Ok(());
    };
    let a = Args::parse(&raw[1..]);
    match cmd.as_str() {
        "list" | "ls" => list(&a),
        "search" => search(&a),
        "info" | "show" => info(&a),
        "add" | "install" => add(&a),
        "configure" | "config" => configure(&a),
        "update" | "upgrade" => update(&a),
        "remove" | "rm" | "uninstall" => remove(&a),
        "enable" => enable(&a),
        "disable" => disable(&a),
        "clone" => clone(&a),
        "new" => new(&a),
        "link" => link(&a),
        "check" => check(&a),
        "pack" => pack(&a),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => bail!("unknown command {other:?}\n\n{USAGE}"),
    }
}

fn find<'a>(found: &'a Found, id: &str) -> Result<&'a Plugin> {
    found.get(id).with_context(|| {
        let near: Vec<&str> = found.plugins.iter().map(|p| p.id()).filter(|p| p.contains(id)).collect();
        if near.is_empty() { format!("no plugin {id} is installed") } else { format!("no plugin {id}; did you mean {}?", near.join(", ")) }
    })
}

fn kind_flag(a: &Args) -> Result<Option<Kind>> {
    match a.get("--kind") {
        Some(k) => Kind::parse(k).map(Some).with_context(|| format!("kind {k:?}: use bar, control-center or desktop")),
        None => Ok(None),
    }
}

fn host_installed(kind: Kind) -> bool {
    discovery::on_path(kind.host_program())
}

// ---------- The running bar ----------

fn bar_bus() -> String {
    std::env::var("NYX_BAR_BUS").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "io.github.design_nexus.nyx.Bar".into())
}

/// What the running bar says about its plugins, by id.
fn live() -> std::collections::HashMap<String, serde_json::Value> {
    let mut out = std::collections::HashMap::new();
    let Ok(conn) = zbus::blocking::Connection::session() else { return out };
    let Ok(reply) = conn.call_method(Some(bar_bus().as_str()), "/io/github/design_nexus/nyx/Bar", Some("io.github.design_nexus.nyx.Bar1"), "ListPlugins", &()) else {
        return out;
    };
    let Ok(json) = reply.body().deserialize::<String>() else { return out };
    if let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(&json) {
        for p in list {
            if let Some(id) = p["id"].as_str() {
                out.insert(id.to_string(), p.clone());
            }
        }
    }
    out
}

// ---------- list, search, info ----------

fn parts_summary(state: &State, m: &Manifest) -> String {
    m.known_kinds()
        .into_iter()
        .map(|k| {
            let on = state.enabled_parts(m, k);
            let shown = if on.is_empty() {
                "off".to_string()
            } else if on == [WHOLE] {
                "on".to_string()
            } else {
                on.join(", ")
            };
            format!("{}: {shown}", k.name())
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn grant_label(state: &State, p: &Plugin) -> &'static str {
    if p.trust.is_trusted() {
        "trusted"
    } else if state.grant_matches(&p.manifest) {
        "approved"
    } else if state.grants.contains_key(p.id()) {
        "permissions changed: needs approval"
    } else {
        "needs approval"
    }
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
}

fn list(a: &Args) -> Result<()> {
    let found = discovery::discover();
    let state = State::load();
    let kind = kind_flag(a)?;
    let live = live();
    let plugins: Vec<&Plugin> = found.plugins.iter().filter(|p| kind.is_none_or(|k| p.manifest.has_kind(k))).collect();
    if a.has("--json") {
        let out: Vec<serde_json::Value> = plugins
            .iter()
            .map(|p| {
                let parts: serde_json::Map<String, serde_json::Value> = p
                    .manifest
                    .known_kinds()
                    .into_iter()
                    .map(|k| (k.name().to_string(), serde_json::json!(state.enabled_parts(&p.manifest, k))))
                    .collect();
                serde_json::json!({
                    "id": p.id(),
                    "name": p.manifest.name,
                    "version": p.manifest.version,
                    "source": p.source_label(),
                    "trust": p.trust.label(),
                    "dir": p.dir,
                    "kinds": p.manifest.kinds,
                    "enabled": parts,
                    "grant": grant_label(&state, p),
                    "suspended": state.suspended.get(p.id()).map(|s| s.reason.clone()),
                    "origin": p.origin.as_ref().map(|o| serde_json::json!({"source": o.source, "commit": o.commit, "index": o.index})),
                    "bar": live.get(p.id()),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    if plugins.is_empty() {
        println!("No plugins installed. Find some with `nyx plugin search`.");
        return Ok(());
    }
    for p in plugins {
        println!("{}  {} {}  ({}, {})", p.id(), p.manifest.name, p.manifest.version, p.source_label(), grant_label(&state, p));
        println!("    {}", parts_summary(&state, &p.manifest));
        if let Some(s) = state.suspended.get(p.id()) {
            println!("    off: {} — turn it back on with `nyx plugin enable {}`", s.reason, p.id());
        }
        let missing = p.missing_needs();
        if !missing.is_empty() {
            println!("    needs {} (not installed)", missing.join(", "));
        }
        if let Some(l) = live.get(p.id()) {
            let mut bits = vec![l["status"].as_str().unwrap_or("").to_string()];
            if let Some(m) = l["memory_bytes"].as_u64() {
                bits.push(mb(m));
            }
            if let Some(c) = l["cpu_percent"].as_f64() {
                bits.push(format!("{c:.1}% CPU"));
            }
            if l["sandboxed"].as_bool() == Some(true) {
                bits.push("sandboxed".into());
            }
            if let Some(r) = l["restarts"].as_u64().filter(|r| *r > 0) {
                bits.push(format!("{r} restarts"));
            }
            if let Some(lim) = l.get("limits").filter(|l| !l.is_null()) {
                bits.push(format!("limits {} MB, {}% CPU", lim["memory_mb"], lim["cpu_percent"]));
            }
            println!("    bar: {}", bits.join(" · "));
        }
    }
    for pr in &found.problems {
        println!("! {}: {}", pr.dir.display(), pr.error);
    }
    Ok(())
}

fn search(a: &Args) -> Result<()> {
    let idx = index::load(a.has("--refresh"))?;
    let found = discovery::discover();
    let term = a.pos.join(" ");
    let hits = idx.search(&term);
    if hits.is_empty() {
        println!("Nothing in the index matches {term:?}.");
    }
    for e in hits {
        let mark = if found.get(&e.id).is_some() { " (installed)" } else { "" };
        let reviewed = if e.reviewed { "reviewed" } else { "not reviewed" };
        println!("{}  {}{mark}\n    {} · {} · {}\n    {}", e.id, e.name, e.kinds.join(", "), e.author, reviewed, e.description);
    }
    Ok(())
}

fn describe(m: &Manifest, trust: Option<Trust>, reviewed: Option<bool>) {
    println!("{} ({}) {}", m.name, m.id, m.version);
    if !m.author.is_empty() || !m.license.is_empty() {
        println!("  by {} · {}", if m.author.is_empty() { "unknown" } else { &m.author }, m.license);
    }
    if !m.description.is_empty() {
        println!("  {}", m.description);
    }
    if let Some(r) = reviewed {
        println!("  {}", if r { "Reviewed for the Nyx plugin index" } else { "Not reviewed" });
    }
    for k in m.known_kinds() {
        let parts = m.parts(k);
        let names: Vec<String> = parts.iter().map(|p| if p.default { format!("{} (on by default)", p.name) } else { p.name.clone() }).collect();
        println!("  {}: {}", k.title(), names.join(", "));
    }
    if !m.needs.is_empty() {
        println!("  Needs: {}", m.needs.join(", "));
    }
    if m.artifact().is_some() {
        println!("  Ships a prebuilt program for {} (its hash is checked)", nyx_plugin::manifest::platform());
    }
    if trust.is_none_or(|t| !t.is_trusted()) {
        println!("  It runs in a sandbox, and may:");
        for line in permissions::describe(&m.permissions) {
            println!("    • {line}");
        }
    }
}

fn info(a: &Args) -> Result<()> {
    let what = a.pos.first().context("which plugin?")?;
    let found = discovery::discover();
    if let Some(p) = found.get(what) {
        describe(&p.manifest, Some(p.trust), None);
        println!("  Installed: {} ({})", p.dir.display(), p.source_label());
        if let Some(o) = &p.origin {
            println!("  From: {} at {}", o.source, &o.commit[..o.commit.len().min(12)]);
        }
        return Ok(());
    }
    if install::check_url(what).is_ok() {
        let staged = install::stage(&Source::url(what, a.get("--ref")))?;
        describe(&staged.manifest, None, None);
        return Ok(());
    }
    let idx = index::load(false)?;
    let e = idx.find(what).with_context(|| format!("{what} isn't installed or in the index"))?;
    let staged = install::stage(&Source::index(e))?;
    describe(&staged.manifest, None, Some(e.reviewed));
    Ok(())
}

// ---------- Choosing parts and placing them ----------

/// The parts the flags pick for `kind`, or None when the flags don't say.
fn flag_parts(a: &Args, m: &Manifest, kind: Kind) -> Option<Vec<String>> {
    let all: Vec<String> = m.parts(kind).into_iter().map(|p| p.id).collect();
    if a.has("--none") {
        return Some(vec![]);
    }
    if a.has("--all") {
        return Some(all);
    }
    let flag = format!("--{}", kind.name());
    if a.has(&flag) {
        return Some(match a.get(&flag) {
            Some(list) => list.split(',').map(|s| s.trim().to_string()).filter(|s| all.contains(s)).collect(),
            None => all,
        });
    }
    if a.has("--enable") {
        return Some(m.default_parts(kind));
    }
    None
}

fn any_part_flag(a: &Args) -> bool {
    ["--none", "--all", "--enable", "--bar", "--control-center", "--desktop"].iter().any(|f| a.has(f))
}

/// The chooser: one checklist over every kind's parts. Returns the choice per kind.
fn choose_parts(m: &Manifest, current: &State, fresh: bool) -> Result<Vec<(Kind, Vec<String>)>> {
    let mut options = vec![];
    let mut on = vec![];
    let mut keys = vec![];
    for k in m.known_kinds() {
        let enabled = if fresh { m.default_parts(k) } else { current.enabled_parts(m, k) };
        for p in m.parts(k) {
            let mut label = format!("{} · {}", k.title(), p.name);
            if !p.description.is_empty() {
                label.push_str(&format!(" — {}", p.description));
            }
            if !host_installed(k) {
                label.push_str(&format!(" ({} isn't installed; saved for later)", k.title()));
            }
            on.push(enabled.contains(&p.id));
            options.push(label);
            keys.push((k, p.id));
        }
    }
    let picked = if options.len() == 1 {
        if ui::confirm(&format!("Turn on {}?", m.name), false)? { vec![0] } else { vec![] }
    } else {
        ui::choose_many(&format!("Which parts of {} do you want?", m.name), &options, &on)?
    };
    Ok(m.known_kinds()
        .into_iter()
        .map(|k| {
            let parts = keys.iter().enumerate().filter(|(i, (kk, _))| *kk == k && picked.contains(i)).map(|(_, (_, p))| p.clone()).collect();
            (k, parts)
        })
        .collect())
}

fn entry_for(m: &Manifest, part: &str) -> String {
    if part == WHOLE || m.section(Kind::Bar).items.len() <= 1 { m.id.clone() } else { format!("{}/{part}", m.id) }
}

fn position(a: &Args) -> Result<Position> {
    Ok(if let Some(b) = a.get("--before") {
        Position::Before(b.into())
    } else if let Some(x) = a.get("--after") {
        Position::After(x.into())
    } else if let Some(i) = a.get("--index") {
        Position::Index(i.parse().context("--index takes a number")?)
    } else {
        Position::End
    })
}

/// Put newly chosen parts in their hosts' layouts.
fn place(a: &Args, m: &Manifest, choice: &[(Kind, Vec<String>)], ask: bool) -> Result<()> {
    for (k, parts) in choice {
        if parts.is_empty() {
            continue;
        }
        let file = paths::layout_file(*k);
        match k {
            Kind::Bar => {
                let mut doc = layout::read_bar(&file)?;
                let default = m.section(Kind::Bar).section.unwrap_or_else(|| "right".into());
                let sections = ["left", "center", "right"];
                for part in parts {
                    let entry = entry_for(m, part);
                    if layout::bar_is_placed(&doc, &entry) && a.get("--section").is_none() {
                        continue;
                    }
                    let section = match a.get("--section") {
                        Some(s) if sections.contains(&s) => s.to_string(),
                        Some(s) => bail!("section {s:?}: use left, center or right"),
                        None if ask && ui::interactive() => {
                            let name = m.parts(Kind::Bar).into_iter().find(|p| p.id == *part).map(|p| p.name).unwrap_or_default();
                            let d = sections.iter().position(|s| *s == default).unwrap_or(2);
                            sections[ui::choose_one(&format!("Where should {name} go in the bar?"), &sections, d)?].to_string()
                        }
                        None => default.clone(),
                    };
                    layout::bar_place(&mut doc, a.get("--monitor"), &entry, &section, &position(a)?);
                }
                layout::write(&file, &doc)?;
            }
            Kind::ControlCenter => {
                let mut doc = layout::read(&file)?;
                for part in parts {
                    let size = m.parts(*k).into_iter().find(|p| p.id == *part).and_then(|p| p.extra.get("sizes").and_then(|s| s.as_array()).and_then(|s| s.first()).and_then(|s| s.as_str()).map(String::from)).unwrap_or_else(|| "2x1".into());
                    layout::cc_append(&mut doc, &layout::tile_id(&m.id, part), &size);
                }
                layout::write(&file, &doc)?;
            }
            Kind::Desktop => {
                let mut doc = layout::read(&file)?;
                for part in parts {
                    let size = m.parts(*k).into_iter().find(|p| p.id == *part).and_then(|p| p.extra.get("sizes").and_then(|s| s.as_array()).and_then(|s| s.first()).and_then(|s| s.as_str()).map(String::from)).unwrap_or_else(|| "medium".into());
                    layout::desktop_append(&mut doc, &format!("{}/{part}", m.id), &size);
                }
                layout::write(&file, &doc)?;
            }
        }
    }
    Ok(())
}

fn save_choice(m: &Manifest, choice: &[(Kind, Vec<String>)]) -> Result<()> {
    let mut e = Editor::open()?;
    for (k, parts) in choice {
        e.set_enabled(&m.id, *k, parts);
    }
    e.unsuspend(&m.id);
    e.save()
}

/// Ask to approve a third-party plugin's permissions; save the grant.
fn approve(m: &Manifest, reviewed: Option<bool>, yes: bool) -> Result<bool> {
    let strong = permissions::changes_system(&m.permissions);
    let mut text = format!(
        "{} runs as you, in a sandbox, with these permissions:\n{}",
        m.name,
        permissions::describe(&m.permissions).iter().map(|l| format!("  • {l}")).collect::<Vec<_>>().join("\n")
    );
    if reviewed == Some(false) || reviewed.is_none() {
        text.push_str("\nIt hasn't been reviewed for the Nyx plugin index. Only add plugins you trust.");
    }
    if strong {
        text.push_str("\nIt can change system settings.");
    }
    ui::warn(&text);
    if !ui::confirm(&format!("Allow {}?", m.name), yes)? {
        return Ok(false);
    }
    let mut e = Editor::open()?;
    e.grant(m, &now_utc());
    e.save()?;
    Ok(true)
}

// ---------- add, configure, update, remove ----------

fn add(a: &Args) -> Result<()> {
    let what = a.pos.first().context("add what? a git URL or a plugin name from `nyx plugin search`")?;
    let idx_entry: Option<Entry>;
    let staged: Staged = if install::check_url(what).is_ok() {
        idx_entry = None;
        install::stage(&Source::url(what, a.get("--ref")))?
    } else if what.contains("://") || what.contains('@') || what.starts_with('-') || what.contains('/') {
        return install::check_url(what);
    } else {
        let idx = index::load(false)?;
        let e = idx.find(what).cloned().with_context(|| format!("{what} isn't in the plugin index (try `nyx plugin search`)"))?;
        if a.get("--ref").is_some() {
            bail!("--ref is for URLs; index plugins install the commit the index pins");
        }
        let s = install::stage(&Source::index(&e))?;
        idx_entry = Some(e);
        s
    };
    let m = staged.manifest.clone();
    describe(&m, None, idx_entry.as_ref().map(|e| e.reviewed));
    if !staged.missing.is_empty() {
        ui::warn(&format!("{} needs {}, which isn't installed; it won't run until it is.", m.name, staged.missing.join(", ")));
    }
    if !approve(&m, idx_entry.as_ref().map(|e| e.reviewed), a.yes())? {
        bail!("not added");
    }
    let dir = match install::install(staged) {
        Ok(d) => d,
        Err(e) => {
            let mut ed = Editor::open()?;
            ed.revoke(&m.id);
            ed.save()?;
            return Err(e);
        }
    };
    println!("Added {} in {}", m.id, dir.display());
    // Nothing is on until the user says so.
    let choice: Vec<(Kind, Vec<String>)> = if any_part_flag(a) {
        m.known_kinds().into_iter().map(|k| (k, flag_parts(a, &m, k).unwrap_or_default())).collect()
    } else if ui::interactive() && !a.yes() {
        choose_parts(&m, &State::load(), true)?
    } else {
        m.known_kinds().into_iter().map(|k| (k, vec![])).collect()
    };
    save_choice(&m, &choice)?;
    place(a, &m, &choice, !a.yes())?;
    if choice.iter().all(|(_, p)| p.is_empty()) {
        println!("It's off for now. Turn it on with `nyx plugin configure {}`.", m.id);
    } else {
        println!("On: {}", parts_summary(&State::load(), &m));
    }
    Ok(())
}

fn pick_plugin(found: &Found) -> Result<String> {
    let ids: Vec<&str> = found.plugins.iter().map(|p| p.id()).collect();
    if ids.is_empty() {
        bail!("no plugins are installed");
    }
    if !ui::interactive() {
        bail!("which plugin?");
    }
    Ok(ids[ui::choose_one("Which plugin?", &ids, 0)?].to_string())
}

fn configure(a: &Args) -> Result<()> {
    let found = discovery::discover();
    let id = match a.pos.first() {
        Some(i) => i.clone(),
        None => pick_plugin(&found)?,
    };
    let p = find(&found, &id)?;
    let state = State::load();
    if !nyx_plugin::authorized(p, &state) && !approve(&p.manifest, None, a.yes())? {
        bail!("{} stays off until its permissions are approved", p.manifest.name);
    }
    let choice = if any_part_flag(a) {
        p.manifest.known_kinds().into_iter().map(|k| (k, flag_parts(a, &p.manifest, k).unwrap_or_else(|| state.enabled_parts(&p.manifest, k)))).collect()
    } else {
        choose_parts(&p.manifest, &state, false)?
    };
    save_choice(&p.manifest, &choice)?;
    place(a, &p.manifest, &choice, true)?;
    println!("{}: {}", p.id(), parts_summary(&State::load(), &p.manifest));
    Ok(())
}

fn update(a: &Args) -> Result<()> {
    let found = discovery::discover();
    let targets: Vec<&Plugin> = match a.pos.first() {
        Some(id) => {
            let p = find(&found, id)?;
            if p.origin.is_none() {
                bail!("{id} wasn't added from git ({}), so there's nothing to update from", p.source_label());
            }
            vec![p]
        }
        None => found.plugins.iter().filter(|p| p.origin.is_some() && !p.linked).collect(),
    };
    if targets.is_empty() {
        println!("No plugins were added from git.");
        return Ok(());
    }
    let idx = if targets.iter().any(|p| p.origin.as_ref().is_some_and(|o| o.index.is_some())) { Some(index::load(true)?) } else { None };
    let mut failed = false;
    for p in targets {
        if let Err(e) = update_one(a, p, idx.as_ref()) {
            eprintln!("nyx plugin: {}: {e:#}", p.id());
            failed = true;
        }
    }
    if failed {
        bail!("some updates failed");
    }
    Ok(())
}

fn update_one(a: &Args, p: &Plugin, idx: Option<&index::Index>) -> Result<()> {
    let o = p.origin.clone().expect("a git plugin");
    let entry = match &o.index {
        Some(name) => Some(idx.and_then(|i| i.find(name)).cloned().with_context(|| format!("{name} is no longer in the index"))?),
        None => None,
    };
    let staged = match &entry {
        Some(e) => {
            if e.commit == o.commit {
                println!("{} is up to date.", p.id());
                return Ok(());
            }
            install::stage(&Source::index(e))?
        }
        None => install::stage(&Source::url(&o.source, o.git_ref.as_deref()))?,
    };
    if staged.origin.commit == o.commit {
        println!("{} is up to date.", p.id());
        return Ok(());
    }
    println!("Changes in {} ({} → {}):", p.id(), &o.commit[..o.commit.len().min(10)], &staged.origin.commit[..10]);
    ui::page(&install::diff(&p.dir, &staged.dir));
    if !ui::confirm(&format!("Update {}?", p.id()), a.yes())? {
        println!("Skipped {}.", p.id());
        return Ok(());
    }
    let perms_changed = permissions::hash(&staged.manifest.permissions) != permissions::hash(&p.manifest.permissions);
    let new_manifest = staged.manifest.clone();
    install::replace(p, staged)?;
    // Check the new version where it now lives; put the old one back if it's broken.
    let check = Manifest::load(&p.dir).and_then(|m| install::check_package(&p.dir, &m));
    if let Err(e) = check {
        install::rollback(p.id())?;
        bail!("the new version is broken ({e:#}); kept the one you had");
    }
    if perms_changed {
        let mut ed = Editor::open()?;
        ed.revoke(p.id());
        ed.save()?;
        println!("{} now asks for different permissions, so it's off until you approve them.", p.id());
        if ui::interactive() && !approve(&new_manifest, entry.as_ref().map(|e| e.reviewed), a.yes())? {
            println!("Approve them later with `nyx plugin enable {}`.", p.id());
        }
    }
    println!("Updated {}.", p.id());
    Ok(())
}

fn unplace_everywhere(id: &str) -> Result<()> {
    let bar = paths::layout_file(Kind::Bar);
    if bar.exists() {
        let mut doc = layout::read(&bar)?;
        layout::bar_unplace(&mut doc, id);
        layout::write(&bar, &doc)?;
    }
    let cc = paths::layout_file(Kind::ControlCenter);
    if cc.exists() {
        let mut doc = layout::read(&cc)?;
        layout::cc_remove(&mut doc, id, None);
        layout::write(&cc, &doc)?;
    }
    let dw = paths::layout_file(Kind::Desktop);
    if dw.exists() {
        let mut doc = layout::read(&dw)?;
        layout::desktop_remove(&mut doc, id, None);
        layout::write(&dw, &doc)?;
    }
    Ok(())
}

fn remove(a: &Args) -> Result<()> {
    let found = discovery::discover();
    let id = a.pos.first().context("remove which plugin?")?;
    let p = find(&found, id)?;
    if p.trust.is_trusted() {
        bail!("{id} is {}; turn it off with `nyx plugin disable {id}`", p.trust.label());
    }
    let what = if p.linked { "Unlink" } else if p.origin.is_some() { "Delete" } else { "Remove (a backup is kept)" };
    if !ui::confirm(&format!("{what} {id}?"), a.yes())? {
        bail!("not removed");
    }
    let mut e = Editor::open()?;
    e.forget(id);
    e.save()?;
    unplace_everywhere(id)?;
    match install::remove(p)? {
        Removed::Unlinked => println!("Unlinked {id}."),
        Removed::Deleted => println!("Removed {id}."),
        Removed::BackedUp(b) => println!("Removed {id}. Backup: {}", b.display()),
    }
    // A packaged copy with the same id takes over again.
    if let Some(under) = discovery::discover().get(id) {
        println!("The {} {id} is back in use.", under.trust.label());
    }
    Ok(())
}

// ---------- enable, disable, clone ----------

fn split_target(t: &str) -> (&str, Option<&str>) {
    match t.split_once('/') {
        Some((p, i)) => (p, Some(i)),
        None => (t, None),
    }
}

fn enable(a: &Args) -> Result<()> {
    let target = a.pos.first().context("enable what? ID or ID/ITEM")?;
    let (id, part) = split_target(target);
    let found = discovery::discover();
    let p = find(&found, id)?;
    let state = State::load();
    if !nyx_plugin::authorized(p, &state) && !approve(&p.manifest, None, a.yes())? {
        bail!("{id} stays off until its permissions are approved");
    }
    let kinds = match kind_flag(a)? {
        Some(k) => vec![k],
        None => p.manifest.known_kinds(),
    };
    let mut choice = vec![];
    for k in kinds {
        let mut parts = state.enabled_parts(&p.manifest, k);
        match part {
            Some(x) => {
                if !p.manifest.parts(k).iter().any(|pp| pp.id == x) {
                    if kind_flag(a)?.is_some() {
                        bail!("{id} has no {} part {x:?}", k.name());
                    }
                    continue;
                }
                if !parts.iter().any(|pp| pp == x) {
                    parts.push(x.to_string());
                }
            }
            None if parts.is_empty() => parts = p.manifest.default_parts(k),
            None => {}
        }
        choice.push((k, parts));
    }
    if choice.is_empty() {
        bail!("{id} has no part {:?}", part.unwrap_or(""));
    }
    save_choice(&p.manifest, &choice)?;
    // Only the named part moves; placement flags apply to the bar.
    let to_place: Vec<(Kind, Vec<String>)> = choice
        .iter()
        .map(|(k, parts)| (*k, parts.iter().filter(|pp| part.is_none_or(|x| x == pp.as_str())).cloned().collect()))
        .collect();
    place(a, &p.manifest, &to_place, false)?;
    println!("{}: {}", id, parts_summary(&State::load(), &p.manifest));
    Ok(())
}

fn disable(a: &Args) -> Result<()> {
    let target = a.pos.first().context("disable what? ID or ID/ITEM")?;
    let (id, part) = split_target(target);
    let found = discovery::discover();
    let p = find(&found, id)?;
    let state = State::load();
    let kinds = match kind_flag(a)? {
        Some(k) => vec![k],
        None => p.manifest.known_kinds(),
    };
    let mut e = Editor::open()?;
    for k in kinds {
        let parts: Vec<String> = match part {
            Some(x) => state.enabled_parts(&p.manifest, k).into_iter().filter(|pp| pp != x).collect(),
            None => vec![],
        };
        e.set_enabled(id, k, &parts);
    }
    e.save()?;
    println!("{}: {}", id, parts_summary(&State::load(), &p.manifest));
    Ok(())
}

fn clone(a: &Args) -> Result<()> {
    let id = a.pos.first().context("clone which plugin?")?;
    let found = discovery::discover();
    let p = find(&found, id)?;
    let dir = install::clone_plugin(p, a.pos.get(1).map(String::as_str))?;
    let new_id = a.pos.get(1).unwrap_or(id);
    println!("Copied {id} to {}. Change it there.", dir.display());
    println!("Your copy is a third-party plugin: it runs sandboxed. Approve and turn it on with `nyx plugin enable {new_id}`.");
    if new_id == id {
        println!("It replaces the {} one while it's there; `nyx plugin remove {id}` brings that back.", p.trust.label());
    }
    Ok(())
}

// ---------- For authors ----------

fn new(a: &Args) -> Result<()> {
    let id = a.pos.first().context("new plugin id? e.g. me.ping")?;
    if !nyx_plugin::manifest::valid_id(id) {
        bail!("{id:?}: use letters, digits, dots and dashes, like me.ping");
    }
    let kind = if a.has("--py") {
        "py"
    } else if a.has("--rust") {
        "rust"
    } else {
        "sh"
    };
    let dir = a.pos.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(id));
    templates::create(&dir, id, kind)?;
    println!("Made {} in {}.", id, dir.display());
    println!("Try it: nyx plugin check {} && nyx plugin link {}", dir.display(), dir.display());
    Ok(())
}

fn link(a: &Args) -> Result<()> {
    let path = a.pos.first().context("link which folder?")?;
    let m = install::link(Path::new(path))?;
    let mut e = Editor::open()?;
    for k in m.known_kinds() {
        e.set_enabled(&m.id, k, &[]);
    }
    e.save()?;
    println!("Linked {} → {}. Edits there restart it.", m.id, path);
    if ui::interactive() && !a.yes() && approve(&m, None, false)? {
        let choice = choose_parts(&m, &State::load(), true)?;
        save_choice(&m, &choice)?;
        place(a, &m, &choice, true)?;
        println!("On: {}", parts_summary(&State::load(), &m));
    } else {
        println!("Turn it on with `nyx plugin enable {}`.", m.id);
    }
    Ok(())
}

fn check(a: &Args) -> Result<()> {
    let path = PathBuf::from(a.pos.first().map(String::as_str).unwrap_or("."));
    let m = Manifest::load(&path)?;
    println!("✓ plugin.toml: {} {} ({})", m.id, m.version, m.kinds.join(", "));
    install::check_package(&path, &m)?;
    println!("✓ programs: {}", m.known_kinds().iter().map(|k| m.exec_for(*k).join(" ")).collect::<Vec<_>>().join("; "));
    let missing: Vec<&String> = m.needs.iter().filter(|n| !discovery::on_path(n)).collect();
    if missing.is_empty() {
        println!("✓ needs: {}", if m.needs.is_empty() { "nothing".into() } else { m.needs.join(", ") });
    } else {
        println!("✗ needs, not installed here: {}", missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
    }
    if let Ok(entries) = std::fs::read_dir(&path) {
        for e in entries.flatten() {
            if std::fs::symlink_metadata(e.path()).is_ok_and(|m| m.file_type().is_symlink()) {
                println!("✗ {} is a link; installs refuse links", e.path().display());
            }
        }
    }
    println!("Permissions it asks for:");
    for l in permissions::describe(&m.permissions) {
        println!("  • {l}");
    }
    let Some(kind) = m.known_kinds().first().copied() else { return Ok(()) };
    if !missing.is_empty() {
        println!("Skipping the test run until its needs are installed.");
        return Ok(());
    }
    // A short run in the sandbox, as a third-party plugin.
    let plugin = Plugin { manifest: m.clone(), dir: path.canonicalize()?, trust: Trust::ThirdParty, origin: None, linked: false };
    let hello = nyx_plugin_protocol::Hello {
        api_version: nyx_plugin_protocol::API_VERSION,
        kind: kind.name().into(),
        plugin: m.id.clone(),
        items: State::default().enabled_parts(&m, kind).into_iter().chain(m.parts(kind).into_iter().map(|p| p.id)).collect(),
        settings: serde_json::to_value(&m.settings).unwrap_or_default(),
        monitors: vec![],
        theme: Default::default(),
        permissions: vec![],
        frame_socket: None,
        bar_height: Some(32),
    };
    let spec = nyx_plugin_host::Spec {
        limits: nyx_plugin_host::limits::Limits::third_party(),
        plugin,
        kind,
        host: "check".into(),
        hello,
    };
    let (tx, rx) = async_channel::unbounded();
    let handle = nyx_plugin_host::spawn(spec, tx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    let mut got = 0;
    let mut problems = vec![];
    while std::time::Instant::now() < deadline {
        match rx.try_recv() {
            Ok(nyx_plugin_host::Report::Message(msg)) => {
                got += 1;
                if got == 1 {
                    println!("✓ sandboxed run: first message is {}", serde_json::to_value(&msg).ok().and_then(|v| v["type"].as_str().map(String::from)).unwrap_or_default());
                }
                if got >= 3 {
                    break;
                }
            }
            Ok(nyx_plugin_host::Report::Warning(w)) => problems.push(w),
            Ok(nyx_plugin_host::Report::Exited { status, stderr }) => {
                problems.push(format!("it exited ({status}){}", if stderr.is_empty() { String::new() } else { format!(": {stderr}") }));
                if got == 0 {
                    break;
                }
            }
            Ok(nyx_plugin_host::Report::Suspended { reason, .. }) => {
                problems.push(reason);
                break;
            }
            Ok(_) => {}
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
    handle.stop();
    for p in &problems {
        println!("✗ {p}");
    }
    if got == 0 {
        bail!("it sent nothing in 8 seconds");
    }
    println!("✓ {got} message(s) in the sandbox");
    Ok(())
}

fn pack(a: &Args) -> Result<()> {
    use sha2::Digest;
    let path = PathBuf::from(a.pos.first().map(String::as_str).unwrap_or("."));
    let m = Manifest::load(&path)?;
    install::check_package(&path, &m)?;
    let out = a.get("--out").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("{}-{}-{}.tar.gz", m.id, m.version, nyx_plugin::manifest::platform())));
    let status = std::process::Command::new("tar")
        .args(["--exclude=.git", "--exclude=target", "--exclude=*.tar.gz", "--exclude=.nyx-plugin.toml", "--owner=0", "--group=0", "--numeric-owner", "-czf"])
        .arg(&out)
        .arg("-C")
        .arg(&path)
        .arg(".")
        .status()?;
    if !status.success() {
        bail!("tar failed");
    }
    let hash = format!("{:x}", sha2::Sha256::digest(std::fs::read(&out)?));
    println!("Made {}.\nPublish it (a release asset), then add to plugin.toml:\n", out.display());
    println!("[artifacts]\n{} = {{ url = \"https://…/{}\", sha256 = \"{hash}\" }}", nyx_plugin::manifest::platform(), out.file_name().unwrap_or_default().to_string_lossy());
    Ok(())
}
