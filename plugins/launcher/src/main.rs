//! Nyx Bar launcher: the Nyx mark in the accent colour. A click opens Glimpse (the
//! launcher and search, Super+Space), or Walker where Glimpse isn't installed; right
//! click opens Settings.

use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin, have};
use serde_json::Value;

/// An arc as cubic curves of at most 90° each (the renderer draws no arcs), from angle
/// `a0` to `a1` in radians, continuing the current path.
fn arc(cx: f64, cy: f64, r: f64, a0: f64, a1: f64) -> String {
    let n = ((a1 - a0).abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = (a1 - a0) / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan() * r;
    let mut d = String::new();
    for i in 0..n {
        let (s, e) = (a0 + step * i as f64, a0 + step * (i + 1) as f64);
        let (x0, y0) = (cx + r * s.cos(), cy + r * s.sin());
        let (x1, y1) = (cx + r * e.cos(), cy + r * e.sin());
        d.push_str(&format!(
            "C{:.3} {:.3} {:.3} {:.3} {:.3} {:.3}",
            x0 - k * s.sin(),
            y0 + k * s.cos(),
            x1 + k * e.sin(),
            y1 - k * e.cos(),
            x1,
            y1
        ));
    }
    d
}

/// The Nyx mark (nyx-mark-small.svg): a circle with a smaller one, offset up and left,
/// cut out of it, traced as its outline between the points where the circles cross.
fn crescent() -> String {
    let ((x1, y1, r1), (x2, y2, r2)) = ((16.0f64, 16.0f64, 13.0f64), (14.0f64, 15.4f64, 11.4f64));
    let (dx, dy) = (x2 - x1, y2 - y1);
    let d = dx.hypot(dy);
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h = (r1 * r1 - a * a).sqrt();
    let (mx, my) = (x1 + a * dx / d, y1 + a * dy / d);
    let p = (mx - h * dy / d, my + h * dx / d);
    let q = (mx + h * dy / d, my - h * dx / d);
    let ang = |(px, py): (f64, f64), cx: f64, cy: f64| (py - cy).atan2(px - cx);
    let tau = std::f64::consts::TAU;
    // The outer circle the long way round, away from the cut; the inner one back.
    let (o0, mut o1) = (ang(q, x1, y1), ang(p, x1, y1));
    if o1 > o0 {
        o1 -= tau;
    }
    let (i0, mut i1) = (ang(p, x2, y2), ang(q, x2, y2));
    if i1 < i0 {
        i1 += tau;
    }
    format!("M{:.3} {:.3}", q.0, q.1) + &arc(x1, y1, r1, o0, o1) + &arc(x2, y2, r2, i0, i1) + "Z"
}

fn mark() -> Node {
    vector([0.0, 0.0, 32.0, 32.0], vec![Draw::Path { d: crescent(), fill: Some(color("accent")), stroke: None, width: None }]).size(17, 17)
}

fn command(settings: &Value) -> Vec<String> {
    let set: Vec<String> = settings
        .get("command")
        .and_then(|c| c.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if !set.is_empty() {
        return set;
    }
    if have("nyx-glimpse") || !have("walker") {
        vec!["nyx-glimpse".into(), "--toggle".into()]
    } else {
        vec!["walker".into()]
    }
}

fn launcher(settings: &Value) -> Item {
    let cmd = command(settings);
    let argv: Vec<&str> = cmd.iter().map(String::as_str).collect();
    item("launcher")
        .view(mark())
        .tooltip("Apps and search (Super+Space)\nRight-click: Settings")
        .on_click(&argv)
        .on_right_click(&["nyx-settings"])
}

fn main() {
    let (plugin, inputs) = Plugin::<()>::start();
    plugin.item(launcher(plugin.settings()));
    for input in inputs {
        match input {
            Input::Settings(st) => plugin.item(launcher(&st)),
            Input::Shutdown => break,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crescent_goes_round_the_right() {
        let d = crescent();
        let nums: Vec<f64> = d.split(|c: char| c.is_ascii_alphabetic() || c == ' ').filter_map(|s| s.parse().ok()).collect();
        let xs: Vec<f64> = nums.iter().step_by(2).copied().collect();
        let right = xs.iter().cloned().fold(f64::MIN, f64::max);
        // Control points sit a little outside the circle they trace.
        assert!(right > 29.0 && right < 31.0, "{right}");
        assert!(xs.iter().all(|x| *x > 2.5), "{d}");
    }

    #[test]
    fn a_set_command_wins() {
        let st = serde_json::json!({ "command": ["fuzzel"] });
        assert_eq!(command(&st), vec!["fuzzel".to_string()]);
    }
}
