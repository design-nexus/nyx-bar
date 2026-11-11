//! Pictures marked `tint-if-mono` (apps' tray icons) whose solid pixels are all one
//! neutral colour, an app's white or black icon, are sent as that glyph alone (its
//! outline and shadow left out) with the class `mono`; the bar draws them in the text
//! colour so they show on light and dark themes alike.

use gdk_pixbuf::Pixbuf;
use gdk_pixbuf::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

thread_local! {
    /// Source → the glyph alone as PNG base64, or None for a picture in colour.
    static SEEN: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

/// Replace every such node in a view (as sent to QML) with its glyph, marked `mono`.
pub fn mark(v: &mut Value) {
    if let Some(children) = v.get_mut("children").and_then(Value::as_array_mut) {
        for c in children {
            mark(c);
        }
    }
    let marked = v.get("class").and_then(Value::as_array).is_some_and(|c| c.iter().any(|x| x == "tint-if-mono"));
    if !marked {
        return;
    }
    let kind = v.get("node").and_then(Value::as_str).unwrap_or_default().to_string();
    let source = match kind.as_str() {
        "icon" => v.get("name").and_then(Value::as_str).map(|n| format!("name:{n}")),
        "image" => v
            .get("path")
            .and_then(Value::as_str)
            .map(|p| format!("path:{p}"))
            .or_else(|| v.get("data").and_then(Value::as_str).map(|d| format!("data:{d}"))),
        _ => None,
    };
    let Some(source) = source else { return };
    let glyph = SEEN.with(|s| {
        let mut s = s.borrow_mut();
        if s.len() > 256 {
            s.clear();
        }
        s.entry(source.clone()).or_insert_with(|| load(&source).and_then(|p| glyph(&p))).clone()
    });
    let (Some(data), Some(o)) = (glyph, v.as_object_mut()) else { return };
    if kind == "icon" {
        let size = o.get("size").and_then(Value::as_i64).unwrap_or(16);
        o.insert("width".into(), size.into());
        o.insert("height".into(), size.into());
    }
    o.insert("node".into(), "image".into());
    o.remove("name");
    o.remove("size");
    o.remove("path");
    o.insert("data".into(), data.into());
    if let Some(c) = o.get_mut("class").and_then(Value::as_array_mut) {
        c.push("mono".into());
    }
}

fn load(source: &str) -> Option<Pixbuf> {
    if let Some(d) = source.strip_prefix("data:") {
        let bytes = glib::base64_decode(d);
        let loader = gdk_pixbuf::PixbufLoader::new();
        loader.write(&bytes).ok()?;
        loader.close().ok()?;
        return loader.pixbuf();
    }
    let file = match source.strip_prefix("path:") {
        Some(p) => PathBuf::from(p),
        None => find_icon(source.strip_prefix("name:")?)?,
    };
    // A drawing at 64 px; a picture at its own size, as scaling blends its edge into
    // its shadow as greys.
    let (_, w, h) = Pixbuf::file_info(&file)?;
    let svg = file.extension().is_some_and(|e| e == "svg");
    if svg || w > 64 || h > 64 { Pixbuf::from_file_at_size(file, 64, 64).ok() } else { Pixbuf::from_file(file).ok() }
}

/// A themed icon's file: the icon themes' apps and status folders, then pixmaps.
fn find_icon(name: &str) -> Option<PathBuf> {
    let name = name.split('|').next()?.trim();
    if name.starts_with('/') {
        return Some(PathBuf::from(name));
    }
    let home = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"));
    let dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    let roots: Vec<PathBuf> = std::iter::once(home)
        .chain(dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from))
        .chain([PathBuf::from("/run/current-system/sw/share")])
        .collect();
    for root in &roots {
        for theme in ["hicolor", "Adwaita"] {
            for size in ["scalable", "48x48", "32x32", "24x24", "22x22", "16x16"] {
                for kind in ["apps", "status"] {
                    for ext in ["svg", "png"] {
                        let p = root.join("icons").join(theme).join(size).join(kind).join(format!("{name}.{ext}"));
                        if p.is_file() {
                            return Some(p);
                        }
                    }
                }
            }
        }
        for ext in ["svg", "png"] {
            let p = root.join("pixmaps").join(format!("{name}.{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn shade(px: &[u8]) -> (f32, bool) {
    let [r, g, b] = [px[0], px[1], px[2]].map(|c| c as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    (0.2126 * r + 0.7152 * g + 0.0722 * b, max == 0.0 || (max - min) / max <= 0.18)
}

/// The shade of the solid pixels when they are all one neutral colour, give or take
/// antialiasing; a soft shadow or outline round the glyph doesn't count.
fn mono_shade(p: &Pixbuf) -> Option<f32> {
    if !p.has_alpha() || p.n_channels() != 4 {
        return None;
    }
    let (w, h, stride) = (p.width() as usize, p.height() as usize, p.rowstride() as usize);
    let bytes = p.read_pixel_bytes();
    let (mut lo, mut hi, mut n) = (1f32, 0f32, 0usize);
    for y in 0..h {
        for x in 0..w {
            let i = y * stride + x * 4;
            let px = bytes.get(i..i + 4)?;
            if px[3] < 220 {
                continue;
            }
            let (l, grey) = shade(px);
            if !grey {
                return None;
            }
            lo = lo.min(l);
            hi = hi.max(l);
            n += 1;
        }
    }
    (n > 0 && hi - lo < 0.3).then_some((lo + hi) / 2.0)
}

#[cfg(test)]
fn is_mono(p: &Pixbuf) -> bool {
    mono_shade(p).is_some()
}

/// The glyph alone, in white with its own alpha, as PNG base64: pixels far from its
/// shade (its outline, its shadow) are left out.
fn glyph(p: &Pixbuf) -> Option<String> {
    let mid = mono_shade(p)?;
    let (w, h, stride) = (p.width() as usize, p.height() as usize, p.rowstride() as usize);
    let src = p.read_pixel_bytes();
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let px = src.get(y * stride + x * 4..y * stride + x * 4 + 4)?;
            let (l, _) = shade(px);
            if px[3] > 0 && (l - mid).abs() < 0.3 {
                out[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[255, 255, 255, px[3]]);
            }
        }
    }
    let clean = Pixbuf::from_bytes(&glib::Bytes::from_owned(out), gdk_pixbuf::Colorspace::Rgb, true, 8, w as i32, h as i32, w as i32 * 4);
    let png = clean.save_to_bufferv("png", &[]).ok()?;
    Some(glib::base64_encode(&png).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixbuf(pixels: &[[u8; 4]]) -> Pixbuf {
        let bytes: Vec<u8> = pixels.iter().flatten().copied().collect();
        Pixbuf::from_bytes(&glib::Bytes::from_owned(bytes), gdk_pixbuf::Colorspace::Rgb, true, 8, pixels.len() as i32, 1, pixels.len() as i32 * 4)
    }

    #[test]
    fn white_icons_are_mono_and_coloured_ones_are_not() {
        assert!(is_mono(&pixbuf(&[[255, 255, 255, 255], [250, 250, 250, 255], [0, 0, 0, 20]])));
        assert!(is_mono(&pixbuf(&[[255, 255, 255, 255], [20, 20, 20, 160]])), "a soft dark outline");
        assert!(!is_mono(&pixbuf(&[[255, 255, 255, 255], [0, 100, 255, 255]])));
        assert!(!is_mono(&pixbuf(&[[255, 255, 255, 255], [20, 20, 20, 255]])), "white and black, both solid");
        assert!(!is_mono(&pixbuf(&[[0, 0, 0, 0]])), "nothing solid");
    }

    #[test]
    fn dropbox_white_tray_icon_is_mono() {
        let f = "/opt/dropbox/images/hicolor/16x16/status/dropboxstatus-idle.png";
        if std::path::Path::new(f).is_file() {
            assert!(is_mono(&load(&format!("path:{f}")).unwrap()));
        }
    }

    #[test]
    fn a_marked_view_gets_its_glyph() {
        let dir = std::env::temp_dir().join(format!("nyx-mono-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("white.png");
        pixbuf(&[[255, 255, 255, 255], [255, 255, 255, 255], [10, 10, 10, 150], [0, 0, 0, 0]]).savev(&file, "png", &[]).unwrap();
        let mut v = serde_json::json!({"node": "box", "children": [
            {"node": "image", "path": file.to_string_lossy(), "class": ["tint-if-mono"]},
            {"node": "image", "path": file.to_string_lossy()},
        ]});
        mark(&mut v);
        let marked = &v["children"][0];
        assert_eq!(marked["class"], serde_json::json!(["tint-if-mono", "mono"]));
        assert!(marked.get("path").is_none());
        let g = load(&format!("data:{}", marked["data"].as_str().unwrap())).unwrap();
        let px = g.read_pixel_bytes();
        assert_eq!(&px[8..12], &[0, 0, 0, 0], "the outline is left out");
        assert_eq!(px[3], 255);
        assert!(v["children"][1].get("class").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
