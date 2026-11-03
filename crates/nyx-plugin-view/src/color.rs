//! Theme tokens. Plugins name colours by token so they follow the theme; the host maps
//! them to the shared stylesheet's colours (for CSS) or to RGBA (for drawings).

use gtk::gdk;

/// Tokens and the stylesheet colour each stands for.
const CSS: &[(&str, &str)] = &[
    ("bg", "@theme_bg"),
    ("surface", "@theme_surface"),
    ("text", "@theme_text"),
    ("dim_text", "@theme_dim_text"),
    ("accent", "@theme_accent"),
    ("on_accent", "@theme_on_accent"),
    ("border", "@theme_border"),
    ("muted", "@theme_muted"),
    ("highlight", "@theme_highlight"),
    ("danger", "@theme_danger"),
    ("panel", "@theme_panel"),
    ("card", "@theme_card"),
    ("raised", "@theme_raised"),
    ("popover", "@theme_popover"),
    ("text_tertiary", "@theme_text_tertiary"),
    ("divider", "@theme_divider"),
    ("warning", "@theme_warning"),
    ("success", "@theme_success"),
    ("hover", "@row_hover"),
    ("field", "@field"),
    ("track", "@track"),
    ("hairline", "@hairline"),
];

fn is_hex(s: &str) -> bool {
    s.starts_with('#') && matches!(s.len(), 4 | 7 | 9) && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// A CSS colour expression for a token or hex colour; None for anything else (so a
/// plugin can't inject CSS through a colour).
pub fn css(token: &str) -> Option<String> {
    let t = token.trim();
    if is_hex(t) {
        return Some(t.to_string());
    }
    CSS.iter().find(|(k, _)| *k == t).map(|(_, v)| v.to_string())
}

fn parse(s: &str) -> Option<gdk::RGBA> {
    gdk::RGBA::parse(s).ok()
}

fn mix(a: gdk::RGBA, b: gdk::RGBA, t: f32) -> gdk::RGBA {
    gdk::RGBA::new(
        a.red() + (b.red() - a.red()) * t,
        a.green() + (b.green() - a.green()) * t,
        a.blue() + (b.blue() - a.blue()) * t,
        a.alpha() + (b.alpha() - a.alpha()) * t,
    )
}

/// A token or hex colour as RGBA, from the current theme.
pub fn rgba(token: &str) -> Option<gdk::RGBA> {
    let t = token.trim();
    if is_hex(t) {
        return parse(t);
    }
    let p = nyx_theme::palette();
    let l = nyx_theme::look();
    let text = parse(&p.text)?;
    let alpha = |c: gdk::RGBA, a: f32| gdk::RGBA::new(c.red(), c.green(), c.blue(), a);
    Some(match t {
        "bg" => parse(&p.bg)?,
        "surface" => parse(&p.surface)?,
        "text" => text,
        "dim_text" => parse(&p.dim_text)?,
        "accent" => parse(&p.accent)?,
        "on_accent" => if p.light { parse("#ffffff")? } else { parse(&p.bg)? },
        "border" => parse(&p.border)?,
        "muted" => parse(&p.muted)?,
        "highlight" => parse(&p.highlight)?,
        "danger" => parse(&p.danger)?,
        "panel" => parse(&l.panel)?,
        "card" => parse(&l.card)?,
        "raised" => parse(&l.raised)?,
        "popover" => parse(&l.popover)?,
        "text_tertiary" => parse(&l.text_tertiary)?,
        "divider" => parse(&l.divider)?,
        "warning" => parse(&l.warning)?,
        "success" => parse(&l.success)?,
        "hover" => mix(parse(&p.bg)?, parse(&p.accent)?, l.hover as f32),
        "field" => alpha(text, 0.035),
        "track" => alpha(text, 0.1),
        "hairline" => mix(parse(&p.bg)?, parse(&p.border)?, 0.82),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_tokens_and_hex() {
        assert_eq!(css("accent").as_deref(), Some("@theme_accent"));
        assert_eq!(css("#ff0000").as_deref(), Some("#ff0000"));
        assert_eq!(css("#ff000080").as_deref(), Some("#ff000080"));
        assert_eq!(css("red; } * { color: red"), None);
        assert_eq!(css("url(file:///etc/passwd)"), None);
    }
}
