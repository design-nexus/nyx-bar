//! A plugin's `style.css` for the Quickshell bar: the rules QML can apply, as JSON.
//!
//! Selectors are descendant chains of compounds: an optional node type (`button`,
//! `image`, `label`, `separator`, `levelbar`, `box`), classes, `:hover`, `:not(.x)`,
//! and `&` for the item's own root. Anything else (`:first-child`, `>`-only meanings,
//! popover internals) is left out, as are properties QML doesn't draw. Colours keep
//! their theme names (`@theme_text` → `text`), so a theme change needs no recompile.

use serde_json::{Map, Value, json};

/// Each rule: `{sel: [compound…], spec, props}`, in the file's order.
pub fn compile(css: &str) -> Value {
    let mut rules = vec![];
    for (prelude, body) in blocks(css) {
        if prelude.starts_with('@') {
            continue;
        }
        let props = declarations(&body);
        if props.is_empty() {
            continue;
        }
        for sel in prelude.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let Some(chain) = selector(sel) else { continue };
            let spec: usize = chain.iter().map(|c| c.spec).sum();
            let compounds: Vec<Value> = chain.into_iter().map(|c| c.json).collect();
            rules.push(json!({ "sel": compounds, "spec": spec, "props": Value::Object(props.clone()) }));
        }
    }
    Value::Array(rules)
}

/// The user's `~/.config/nyx/bar.css`, as the GTK bar reads it: rules for every item
/// (their selectors may name an item's classes, `.bar-item`, `.plugin-<id>`), and the
/// bar's own parts by their GTK names: the bar (`window.nyx-bar`, `.bar-sections`),
/// pills (`.bar-group`) and dividers (`.bar-divider`). Pills aren't tinted while open, so
/// `.bar-group.open` is left out.
pub fn user(css: &str) -> Value {
    let (mut items, mut bar, mut group, mut divider) = (vec![], Map::new(), Map::new(), Map::new());
    for (prelude, body) in blocks(css) {
        if prelude.starts_with('@') {
            continue;
        }
        let props = declarations(&body);
        if props.is_empty() {
            continue;
        }
        for sel in prelude.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            // The window around everything says nothing for QML's rules.
            let parts: Vec<&str> =
                sel.split_whitespace().filter(|p| *p != ">").skip_while(|p| matches!(*p, "window.nyx-bar" | ".nyx-bar" | "window")).collect();
            if matches!(parts.as_slice(), [p] if p.starts_with(".bar-group") && p.contains(".open")) {
                continue;
            }
            let into = match parts.as_slice() {
                [] | [".bar-sections"] => Some(&mut bar),
                [p] if p.starts_with(".bar-group") => Some(&mut group),
                [p] if p.starts_with(".bar-divider") => Some(&mut divider),
                _ => None,
            };
            if let Some(m) = into {
                m.extend(props.clone());
                continue;
            }
            let Some(chain) = selector(&parts.join(" ")) else { continue };
            let spec: usize = chain.iter().map(|c| c.spec).sum();
            let compounds: Vec<Value> = chain.into_iter().map(|c| c.json).collect();
            items.push(json!({ "sel": compounds, "spec": spec, "props": Value::Object(props.clone()) }));
        }
    }
    json!({ "items": items, "bar": bar, "group": group, "divider": divider })
}

fn blocks(css: &str) -> Vec<(String, String)> {
    let mut text = String::new();
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        text.push_str(&rest[..i]);
        rest = rest[i + 2..].find("*/").map_or("", |j| &rest[i + 2 + j + 2..]);
    }
    text.push_str(rest);
    // Blocks nest (`@keyframes x { from { … } }`): each top-level one ends at its
    // matching brace, so the rules after it survive.
    let mut out = vec![];
    let mut cur = text.as_str();
    while let Some(open) = cur.find('{') {
        let mut depth = 0;
        let Some(close) = cur[open..].char_indices().find_map(|(i, c)| {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
            None
        }) else {
            break;
        };
        out.push((cur[..open].trim().to_string(), cur[open + 1..open + close].to_string()));
        cur = &cur[open + close + 1..];
    }
    out
}

/// `s` split on commas outside parentheses.
fn args(s: &str) -> Vec<&str> {
    let (mut out, mut depth, mut start) = (vec![], 0, 0);
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(s[start..].trim());
    out
}

struct Compound {
    json: Value,
    spec: usize,
}

fn selector(sel: &str) -> Option<Vec<Compound>> {
    let mut chain = vec![];
    for part in sel.split_whitespace() {
        if part == ">" {
            continue;
        }
        chain.push(compound(part)?);
    }
    (!chain.is_empty()).then_some(chain)
}

fn ident(s: &str) -> (&str, &str) {
    let end = s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_')).unwrap_or(s.len());
    (&s[..end], &s[end..])
}

fn compound(s: &str) -> Option<Compound> {
    let mut rest = s;
    let mut root = false;
    let mut kind: Option<&str> = None;
    if let Some(r) = rest.strip_prefix('&') {
        root = true;
        rest = r;
    } else if rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
        let (t, r) = ident(rest);
        kind = Some(match t {
            "button" => "button",
            "image" => "image",
            "label" => "label",
            "separator" => "separator",
            "levelbar" => "progress",
            "box" => "box",
            _ => return None,
        });
        rest = r;
    }
    let (mut classes, mut not, mut hover) = (vec![], vec![], false);
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('.') {
            let (c, r) = ident(r);
            classes.push(c.to_string());
            rest = r;
        } else if let Some(r) = rest.strip_prefix(":hover") {
            hover = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix(":not(.") {
            let (c, r) = ident(r);
            not.push(c.to_string());
            rest = r.strip_prefix(')')?;
        } else {
            // :first-child, :focus-visible, block internals: not for QML.
            return None;
        }
    }
    let spec = classes.len() + not.len() + hover as usize + kind.is_some() as usize + root as usize;
    Some(Compound { json: json!({ "type": kind, "classes": classes, "not": not, "hover": hover, "root": root }), spec })
}

/// `@theme_text`, `alpha(@theme_text, 0.06)`, `mix(@a, @b, 0.14)`, `shade(@x, 1.08)`,
/// `#rgb`, `#rrggbb[aa]`, `rgb()`/`rgba()`, a few named colours, `none`/`transparent`.
pub fn color(v: &str) -> Option<Value> {
    let v = v.trim();
    if v == "none" || v == "transparent" {
        return Some(json!({ "none": true }));
    }
    if let Some(name) = v.strip_prefix('@') {
        return Some(json!({ "t": name.strip_prefix("theme_").unwrap_or(name) }));
    }
    if let Some(h) = v.strip_prefix('#') {
        if !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        return match h.len() {
            3 => Some(json!({ "hex": format!("#{}", h.chars().flat_map(|c| [c, c]).collect::<String>()) })),
            6 | 8 => Some(json!({ "hex": v })),
            _ => None,
        };
    }
    let named = match v.to_ascii_lowercase().as_str() {
        "white" => Some("#ffffff"),
        "black" => Some("#000000"),
        "red" => Some("#ff0000"),
        "green" => Some("#008000"),
        "blue" => Some("#0000ff"),
        "yellow" => Some("#ffff00"),
        "orange" => Some("#ffa500"),
        "gray" | "grey" => Some("#808080"),
        _ => None,
    };
    if let Some(hex) = named {
        return Some(json!({ "hex": hex }));
    }
    let (f, rest) = v.split_once('(')?;
    let args = args(rest.strip_suffix(')')?);
    match (f.trim(), args.as_slice()) {
        ("rgb" | "rgba", [r, g, b, a @ ..]) if a.len() <= 1 => {
            let channel = |c: &str| -> Option<u8> {
                match c.strip_suffix('%') {
                    Some(p) => p.trim().parse::<f64>().ok().map(|p| (p.clamp(0.0, 100.0) * 2.55).round() as u8),
                    None => c.parse::<f64>().ok().map(|n| n.clamp(0.0, 255.0).round() as u8),
                }
            };
            let mut c = json!({ "hex": format!("#{:02x}{:02x}{:02x}", channel(r)?, channel(g)?, channel(b)?) });
            if let Some(a) = a.first() {
                c["a"] = json!(a.parse::<f64>().ok()?);
            }
            Some(c)
        }
        ("alpha", [c, a]) => {
            let mut c = color(c)?;
            c["a"] = json!(a.parse::<f64>().ok()?);
            Some(c)
        }
        ("shade", [c, k]) => {
            let mut c = color(c)?;
            c["shade"] = json!(k.parse::<f64>().ok()?);
            Some(c)
        }
        ("mix", [a, b, t]) => Some(json!({ "mix": [color(a)?, color(b)?, t.parse::<f64>().ok()?] })),
        _ => None,
    }
}

fn px(v: &str) -> Option<f64> {
    let v = v.trim();
    if v == "0" {
        return Some(0.0);
    }
    if let Some(n) = v.strip_suffix("px") {
        return n.parse().ok();
    }
    if let Some(n) = v.strip_suffix("em") {
        return n.parse::<f64>().ok().map(|e| e * 13.0);
    }
    v.parse().ok()
}

fn sides(v: &str) -> Option<Value> {
    let n: Vec<f64> = v.split_whitespace().map(px).collect::<Option<_>>()?;
    let [t, r, b, l] = match n.as_slice() {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return None,
    };
    Some(json!([t, r, b, l]))
}

/// `1px solid <colour>` (or dashed): `{w, c, dashed}`.
fn line(v: &str) -> Option<Value> {
    let mut it = v.trim().splitn(3, ' ');
    let w = px(it.next()?)?;
    let style = it.next()?;
    let c = color(it.next()?)?;
    Some(json!({ "w": w, "c": c, "dashed": style == "dashed" }))
}

/// One inset shadow as QML draws it: `("ring", {w, c})` or `("border-<side>", {w, c})`.
fn inset(sh: &str) -> Option<(&'static str, Value)> {
    let rest = sh.trim().strip_prefix("inset ")?;
    let toks: Vec<&str> = rest.split_whitespace().collect();
    let n = toks.iter().take_while(|t| px(t).is_some()).count();
    let lens: Vec<f64> = toks[..n].iter().filter_map(|t| px(t)).collect();
    let c = color(&toks[n..].join(" "))?;
    let (x, y, blur, spread) = match lens.as_slice() {
        [x, y] => (*x, *y, 0.0, 0.0),
        [x, y, b] => (*x, *y, *b, 0.0),
        [x, y, b, s] => (*x, *y, *b, *s),
        _ => return None,
    };
    if blur != 0.0 {
        return None;
    }
    let edge = |k, w: f64| Some((k, json!({ "w": w.abs(), "c": c.clone() })));
    match (x, y, spread) {
        (0.0, 0.0, s) if s > 0.0 => edge("ring", s),
        (0.0, y, 0.0) if y < 0.0 => edge("border-bottom", y),
        (0.0, y, 0.0) if y > 0.0 => edge("border-top", y),
        (x, 0.0, 0.0) if x > 0.0 => edge("border-left", x),
        (x, 0.0, 0.0) if x < 0.0 => edge("border-right", x),
        _ => None,
    }
}

fn declarations(body: &str) -> Map<String, Value> {
    let mut out = Map::new();
    for d in body.split(';').map(str::trim).filter(|d| !d.is_empty()) {
        let Some((k, v)) = d.split_once(':') else { continue };
        let (k, v) = (k.trim(), v.trim());
        let val = match k {
            "padding" | "margin" => sides(v),
            "padding-top" | "padding-right" | "padding-bottom" | "padding-left" | "margin-top" | "margin-right"
            | "margin-bottom" | "margin-left" | "min-width" | "min-height" | "font-size" | "letter-spacing"
            | "-gtk-icon-size" => px(v).map(Value::from),
            "border-radius" => v.split_whitespace().next().and_then(px).map(Value::from),
            "background" | "background-color" => color(v),
            "color" => color(v),
            "opacity" => v.parse::<f64>().ok().map(Value::from),
            "font-weight" => match v {
                "bold" => Some(json!(700)),
                "normal" => Some(json!(400)),
                n => n.parse::<i64>().ok().map(Value::from),
            },
            "box-shadow" if v == "none" => Some(json!({ "none": true })),
            "box-shadow" => {
                // Inset shadows QML can draw: a ring (`inset 0 0 0 1px c`) or a line on
                // one side (`inset 0 -2px 0 c`, the bottom).
                for sh in args(v) {
                    if let Some((key, val)) = inset(sh) {
                        out.insert(key.into(), val);
                    }
                }
                None
            }
            "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => line(v),
            "-gtk-icon-transform" => v.strip_prefix("rotate(").and_then(|r| r.strip_suffix("deg)")).and_then(|d| d.parse::<f64>().ok()).map(Value::from),
            _ => None,
        };
        if let Some(val) = val {
            let key = match k {
                "background" => "background-color".to_string(),
                "box-shadow" => "ring".to_string(),
                "-gtk-icon-size" => "icon-size".to_string(),
                "-gtk-icon-transform" => "rotate".to_string(),
                _ => k.to_string(),
            };
            out.insert(key, val);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_from_a_plugins_css() {
        let r = compile(
            "/* x */ &.bar-item { padding: 0 2px; }\n.ev:hover .open { opacity: 1; }\n.dn.today { background-color: @theme_accent; color: @theme_on_accent; font-weight: 700; }\n\
             button.ibtn image { color: inherit; -gtk-icon-size: 16px; }\n.day:hover .dn:not(.today) { background-color: alpha(@theme_text, 0.07); }\n\
             .a:first-child { color: red; }\n.pi.scr { background-color: alpha(#a78bfa, 0.22); }\n.join.primary:hover { background-color: shade(@theme_accent, 1.08); }",
        );
        let r = r.as_array().unwrap();
        assert_eq!(r[0]["sel"][0]["root"], true);
        assert_eq!(r[0]["props"]["padding"], json!([0.0, 2.0, 0.0, 2.0]));
        assert_eq!(r[1]["sel"][0]["hover"], true);
        assert_eq!(r[1]["sel"][1]["classes"], json!(["open"]));
        assert_eq!(r[2]["props"]["background-color"], json!({ "t": "accent" }));
        assert_eq!(r[3]["sel"][1]["type"], "image");
        assert_eq!(r[3]["props"]["icon-size"], 16.0);
        assert!(r[3]["props"].get("color").is_none(), "inherit isn't a colour");
        assert_eq!(r[4]["sel"][1]["not"], json!(["today"]));
        assert_eq!(r[4]["props"]["background-color"], json!({ "t": "text", "a": 0.07 }));
        assert_eq!(r.len(), 7, ":first-child is left out");
        assert_eq!(r[5]["props"]["background-color"], json!({ "hex": "#a78bfa", "a": 0.22 }));
        assert_eq!(r[6]["props"]["background-color"], json!({ "t": "accent", "shade": 1.08 }));
    }

    #[test]
    fn keyframes_colours_and_inset_shadows() {
        let r = compile(
            ".dot { color: @theme_warning; animation: pulse 1s infinite; }\n@keyframes pulse { from { opacity: 1; } to { opacity: 0.3; } }\n\
             .after { color: #abc; }\n.rgb { background-color: rgba(255, 0, 0, 0.5); color: rgb(0%, 100%, 0%); }\n\
             .named { color: white; }\n.nested { background-color: alpha(rgb(10, 20, 30), 0.2); }\n\
             .under { box-shadow: inset 0 -2px 0 @theme_accent; }\n.edge { box-shadow: inset 3px 0 0 @theme_danger, inset 0 0 0 1px @nav_active; }\n\
             .blur { box-shadow: inset 0 1px 2px black; }\n.side { border-bottom: 1px solid @theme_border; }",
        );
        let r = r.as_array().unwrap();
        let props = |class: &str| r.iter().find(|x| x["sel"][0]["classes"][0] == class).map(|x| x["props"].clone());
        assert_eq!(props("dot").unwrap()["color"], json!({ "t": "warning" }));
        assert_eq!(props("after").unwrap()["color"], json!({ "hex": "#aabbcc" }), "rules after @keyframes survive");
        assert_eq!(props("rgb").unwrap()["background-color"], json!({ "hex": "#ff0000", "a": 0.5 }));
        assert_eq!(props("rgb").unwrap()["color"], json!({ "hex": "#00ff00" }));
        assert_eq!(props("named").unwrap()["color"], json!({ "hex": "#ffffff" }));
        assert_eq!(props("nested").unwrap()["background-color"], json!({ "hex": "#0a141e", "a": 0.2 }));
        assert_eq!(props("under").unwrap()["border-bottom"], json!({ "w": 2.0, "c": { "t": "accent" } }));
        let edge = props("edge").unwrap();
        assert_eq!(edge["border-left"], json!({ "w": 3.0, "c": { "t": "danger" } }));
        assert_eq!(edge["ring"], json!({ "w": 1.0, "c": { "t": "nav_active" } }));
        assert!(props("blur").is_none(), "a blurred shadow isn't drawn");
        assert_eq!(props("side").unwrap()["border-bottom"]["w"], 1.0);
    }

    #[test]
    fn the_users_bar_css() {
        let u = user(
            "window.nyx-bar { background-color: #101010; }\n.bar-group { background-color: alpha(@theme_text, 0.1); border-radius: 6px; }\n\
             .bar-group.open { background-color: @theme_accent; }\n.bar-divider { background-color: #ff0000; }\n\
             .bar-item.plugin-nyx_clock label { font-weight: bold; }\nwindow.nyx-bar .plugin-nyx_weather { padding: 0 8px; }",
        );
        assert_eq!(u["bar"]["background-color"], json!({ "hex": "#101010" }));
        assert_eq!(u["group"]["border-radius"], 6.0);
        assert_eq!(u["group"]["background-color"], json!({ "t": "text", "a": 0.1 }), ".bar-group.open is left out");
        assert!(u.get("groupOpen").is_none());
        assert_eq!(u["divider"]["background-color"], json!({ "hex": "#ff0000" }));
        let items = u["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["sel"][0]["classes"], json!(["bar-item", "plugin-nyx_clock"]));
        assert_eq!(items[0]["sel"][1]["type"], "label");
        assert_eq!(items[1]["sel"][0]["classes"], json!(["plugin-nyx_weather"]));
    }
}
