//! A window's class to the app it belongs to: its name and icon, from the installed
//! `.desktop` files (`$XDG_DATA_HOME` and `$XDG_DATA_DIRS`, `applications/`). The files
//! are read once; a class no file claims keeps its own name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub name: String,
    /// An icon name (or a path, for the rare file that gives one).
    pub icon: String,
}

impl App {
    /// An icon list for the bar's views: the app's symbolic icon, its own, then a generic
    /// one (the view shows the first the icon theme has).
    pub fn icons_symbolic_first(&self) -> String {
        format!("{0}-symbolic|{0}|application-x-executable-symbolic", self.icon)
    }

    pub fn icons(&self) -> String {
        format!("{0}|{0}-symbolic|application-x-executable", self.icon)
    }
}

struct Entry {
    stem: String,
    wm_class: Option<String>,
    name: String,
    icon: String,
}

fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    home.into_iter().chain(dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from)).collect()
}

fn parse(path: &Path) -> Option<Entry> {
    let text = std::fs::read_to_string(path).ok()?;
    let (mut name, mut icon, mut wm_class) = (None, None, None);
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "Name" => name = Some(v.trim().to_string()),
            "Icon" => icon = Some(v.trim().to_string()),
            "StartupWMClass" => wm_class = Some(v.trim().to_lowercase()),
            _ => {}
        }
    }
    let stem = path.file_stem()?.to_string_lossy().to_lowercase();
    Some(Entry { stem, wm_class, name: name?, icon: icon.unwrap_or_default() })
}

fn load(dirs: &[PathBuf]) -> Vec<Entry> {
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d.join("applications")) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "desktop")
                && seen.insert(p.file_name().map(|f| f.to_os_string()))
                && let Some(entry) = parse(&p)
            {
                out.push(entry);
            }
        }
    }
    out
}

fn find<'a>(entries: &'a [Entry], class: &str) -> Option<&'a Entry> {
    let c = class.to_lowercase();
    entries
        .iter()
        .find(|e| e.wm_class.as_deref() == Some(c.as_str()))
        .or_else(|| entries.iter().find(|e| e.stem == c))
        .or_else(|| entries.iter().find(|e| e.stem.rsplit('.').next() == Some(c.as_str())))
        .or_else(|| entries.iter().find(|e| e.name.to_lowercase() == c))
}

fn lookup_in(entries: &[Entry], class: &str) -> App {
    match find(entries, class) {
        Some(e) => App { name: e.name.clone(), icon: if e.icon.is_empty() { class.to_lowercase() } else { e.icon.clone() } },
        None => {
            // org.example.App → App
            let short = class.rsplit('.').next().unwrap_or(class);
            let mut name: String = short.to_string();
            if let Some(f) = name.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            App { name, icon: class.to_lowercase() }
        }
    }
}

static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
static CACHE: OnceLock<std::sync::Mutex<HashMap<String, App>>> = OnceLock::new();

/// The app a window class belongs to.
pub fn lookup(class: &str) -> App {
    let cache = CACHE.get_or_init(Default::default);
    if let Some(a) = cache.lock().ok().and_then(|c| c.get(class).cloned()) {
        return a;
    }
    let app = lookup_in(ENTRIES.get_or_init(|| load(&data_dirs())), class);
    if let Ok(mut c) = cache.lock() {
        c.insert(class.to_string(), app.clone());
    }
    app
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_find_their_desktop_files() {
        let d = tempfile::tempdir().unwrap();
        let apps = d.path().join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(apps.join("io.github.design_nexus.nyx.Files.desktop"), "[Desktop Entry]\nName=Files\nIcon=io.github.design_nexus.nyx.Files\n").unwrap();
        std::fs::write(apps.join("chromium.desktop"), "[Desktop Entry]\nName=Chromium\nIcon=chromium\nStartupWMClass=Chromium-browser\n[Desktop Action new]\nName=New window\n").unwrap();
        let e = load(&[d.path().to_path_buf()]);
        assert_eq!(lookup_in(&e, "io.github.design_nexus.nyx.Files").name, "Files");
        assert_eq!(lookup_in(&e, "chromium-browser").name, "Chromium");
        assert_eq!(lookup_in(&e, "Files").icon, "io.github.design_nexus.nyx.Files", "by the file's last part");
        let none = lookup_in(&e, "org.acme.thing");
        assert_eq!((none.name.as_str(), none.icon.as_str()), ("Thing", "org.acme.thing"));
        assert!(none.icons_symbolic_first().starts_with("org.acme.thing-symbolic|"));
    }
}
