//! A plugin's `style.css`, scoped so it only reaches the plugin's own widgets: every
//! selector gets `.plugin-<id>` in front, `@keyframes` are renamed per plugin, other
//! at-rules (`@define-color`, `@import`) and `url()` are dropped, and only so many
//! infinite animations are kept.

/// The CSS class on everything a plugin draws.
pub fn plugin_class(id: &str) -> String {
    let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    format!("plugin-{safe}")
}

fn split_blocks(css: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut depth = 0;
    let mut prelude = String::new();
    let mut body = String::new();
    let mut in_comment = false;
    let chars: Vec<char> = css.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_comment {
            if c == '*' && chars.get(i + 1) == Some(&'/') {
                in_comment = false;
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            in_comment = true;
            i += 2;
            continue;
        }
        match c {
            '{' => {
                depth += 1;
                if depth > 1 {
                    body.push(c);
                }
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    out.push((prelude.trim().to_string(), body.trim().to_string()));
                    prelude.clear();
                    body.clear();
                } else if depth > 0 {
                    body.push(c);
                } else {
                    depth = 0;
                }
            }
            ';' if depth == 0 => prelude.clear(),
            _ if depth == 0 => prelude.push(c),
            _ => body.push(c),
        }
        i += 1;
    }
    out
}

fn rename_words(text: &str, names: &[(String, String)]) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        match names.iter().find(|(from, _)| from == word) {
            Some((_, to)) => out.push_str(to),
            None => out.push_str(word),
        }
        word.clear();
    };
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// The scoped stylesheet.
pub fn scope(css: &str, plugin_id: &str, max_infinite: usize) -> String {
    let class = plugin_class(plugin_id);
    let blocks = split_blocks(css);
    let names: Vec<(String, String)> = blocks
        .iter()
        .filter_map(|(p, _)| p.strip_prefix("@keyframes").map(|n| n.trim().to_string()))
        .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .map(|n| (n.clone(), format!("{class}-{n}")))
        .collect();
    let mut infinite = 0;
    let mut out = String::new();
    for (prelude, body) in blocks {
        if body.contains("url(") || prelude.contains("url(") {
            continue;
        }
        if let Some(name) = prelude.strip_prefix("@keyframes") {
            if let Some((_, to)) = names.iter().find(|(from, _)| from == name.trim()) {
                out.push_str(&format!("@keyframes {to} {{ {body} }}\n"));
            }
            continue;
        }
        if prelude.starts_with('@') || prelude.is_empty() {
            continue;
        }
        let selectors: Vec<String> = prelude
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| match s {
                "&" | ":root" => format!(".{class}"),
                s if s.starts_with('&') => format!(".{class}{}", &s[1..]),
                s => format!(".{class} {s}"),
            })
            .collect();
        if selectors.is_empty() {
            continue;
        }
        let mut decls = vec![];
        for d in body.split(';').map(str::trim).filter(|d| !d.is_empty()) {
            let Some((prop, value)) = d.split_once(':') else { continue };
            let prop = prop.trim();
            if prop.starts_with("animation") {
                if value.contains("infinite") || prop == "animation-iteration-count" && value.trim() == "infinite" {
                    infinite += 1;
                    if infinite > max_infinite {
                        continue;
                    }
                }
                decls.push(format!("{prop}: {}", rename_words(value.trim(), &names)));
            } else {
                decls.push(format!("{prop}: {}", value.trim()));
            }
        }
        out.push_str(&format!("{} {{ {}; }}\n", selectors.join(", "), decls.join("; ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_selectors_and_keyframes() {
        let css = r#"
/* a comment { } */
@import "x.css";
@define-color theme_accent red;
label { color: @theme_accent; }
& { padding: 2px }
.dot, image:hover { animation: pulse 1s infinite; }
@keyframes pulse { from { opacity: 1 } to { opacity: 0.3 } }
.bg { background-image: url("file:///etc/passwd"); }
"#;
        let out = scope(css, "acme.rec", 4);
        assert!(out.contains(".plugin-acme_rec label { color: @theme_accent; }"), "{out}");
        assert!(out.contains(".plugin-acme_rec { padding: 2px; }"), "{out}");
        assert!(out.contains(".plugin-acme_rec .dot, .plugin-acme_rec image:hover { animation: plugin-acme_rec-pulse 1s infinite; }"), "{out}");
        assert!(out.contains("@keyframes plugin-acme_rec-pulse { from { opacity: 1 } to { opacity: 0.3 } }"), "{out}");
        assert!(!out.contains("@import") && !out.contains("define-color") && !out.contains("url("), "{out}");
    }

    #[test]
    fn limits_infinite_animations() {
        let css = "a { animation: x 1s infinite } b { animation: x 1s infinite } c { animation: x 1s infinite }";
        let out = scope(css, "p", 2);
        assert_eq!(out.matches("infinite").count(), 2);
    }
}
