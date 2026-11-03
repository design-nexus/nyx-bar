//! Vector drawings: drawing commands with cairo, SVG through librsvg.

use crate::color;
use gtk::cairo;
use nyx_plugin_protocol::view::{Linear, Radial};
use nyx_plugin_protocol::{Draw, Paint, Vector};

fn set_paint(cr: &cairo::Context, paint: &Paint) -> bool {
    let stops = |p: &cairo::Gradient, stops: &[(f64, String)]| {
        for (offset, c) in stops {
            if let Some(rgba) = color::rgba(c) {
                p.add_color_stop_rgba(*offset, rgba.red() as f64, rgba.green() as f64, rgba.blue() as f64, rgba.alpha() as f64);
            }
        }
    };
    match paint {
        Paint::Color(c) => match color::rgba(c) {
            Some(rgba) => {
                cr.set_source_rgba(rgba.red() as f64, rgba.green() as f64, rgba.blue() as f64, rgba.alpha() as f64);
                true
            }
            None => false,
        },
        Paint::Linear { linear: Linear { x1, y1, x2, y2, stops: s } } => {
            let g = cairo::LinearGradient::new(*x1, *y1, *x2, *y2);
            stops(&g, s);
            cr.set_source(&g).is_ok()
        }
        Paint::Radial { radial: Radial { cx, cy, r, stops: s } } => {
            let g = cairo::RadialGradient::new(*cx, *cy, 0.0, *cx, *cy, *r);
            stops(&g, s);
            cr.set_source(&g).is_ok()
        }
    }
}

fn finish(cr: &cairo::Context, fill: &Option<Paint>, stroke: &Option<Paint>, width: Option<f64>) {
    if let Some(f) = fill
        && set_paint(cr, f)
    {
        let _ = if stroke.is_some() { cr.fill_preserve() } else { cr.fill() };
    }
    if let Some(s) = stroke
        && set_paint(cr, s)
    {
        cr.set_line_width(width.unwrap_or(1.0));
        let _ = cr.stroke();
    }
    cr.new_path();
}

/// Draw `v` into a `w × h` area.
pub fn draw(cr: &cairo::Context, v: &Vector, w: f64, h: f64) {
    if let Some(svg) = &v.svg {
        draw_svg(cr, svg, w, h);
        return;
    }
    let _ = cr.save();
    if let Some([x, y, vw, vh]) = v.view_box
        && vw > 0.0
        && vh > 0.0
    {
        let s = (w / vw).min(h / vh);
        cr.translate((w - vw * s) / 2.0, (h - vh * s) / 2.0);
        cr.scale(s, s);
        cr.translate(-x, -y);
    }
    for d in &v.commands {
        match d {
            Draw::Path { d, fill, stroke, width } => {
                path(cr, d);
                finish(cr, fill, stroke, *width);
            }
            Draw::Line { x1, y1, x2, y2, stroke, width, round } => {
                cr.set_line_cap(if *round { cairo::LineCap::Round } else { cairo::LineCap::Butt });
                cr.move_to(*x1, *y1);
                cr.line_to(*x2, *y2);
                finish(cr, &None, &stroke.clone().or(Some(Paint::Color("text".into()))), *width);
            }
            Draw::Arc { cx, cy, r, start, end, fill, stroke, width, round } => {
                cr.set_line_cap(if *round { cairo::LineCap::Round } else { cairo::LineCap::Butt });
                cr.arc(*cx, *cy, *r, start.to_radians(), end.to_radians());
                finish(cr, fill, stroke, *width);
            }
            Draw::Circle { cx, cy, r, fill, stroke, width } => {
                cr.arc(*cx, *cy, *r, 0.0, std::f64::consts::TAU);
                finish(cr, fill, stroke, *width);
            }
            Draw::Rect { x, y, w, h, radius, fill, stroke, width } => {
                rounded(cr, *x, *y, *w, *h, *radius);
                finish(cr, fill, stroke, *width);
            }
            Draw::Text { x, y, text, size, fill, align, bold } => {
                cr.select_font_face("Inter", cairo::FontSlant::Normal, if *bold { cairo::FontWeight::Bold } else { cairo::FontWeight::Normal });
                cr.set_font_size(*size);
                let ext = cr.text_extents(text).ok();
                let dx = match (align.as_deref(), ext) {
                    (Some("center"), Some(e)) => -e.width() / 2.0 - e.x_bearing(),
                    (Some("end"), Some(e)) => -e.width() - e.x_bearing(),
                    _ => 0.0,
                };
                cr.move_to(x + dx, *y);
                if set_paint(cr, fill.as_ref().unwrap_or(&Paint::Color("text".into()))) {
                    let _ = cr.show_text(text);
                }
                cr.new_path();
            }
            Draw::Save => {
                let _ = cr.save();
            }
            Draw::Restore => {
                let _ = cr.restore();
            }
            Draw::Translate { x, y } => cr.translate(*x, *y),
            Draw::Rotate { deg } => cr.rotate(deg.to_radians()),
            Draw::Scale { x, y } => cr.scale(*x, *y),
        }
    }
    let _ = cr.restore();
}

pub fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    if r == 0.0 {
        cr.rectangle(x, y, w, h);
        return;
    }
    use std::f64::consts::FRAC_PI_2 as Q;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -Q, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, Q);
    cr.arc(x + r, y + h - r, r, Q, 2.0 * Q);
    cr.arc(x + r, y + r, r, 2.0 * Q, 3.0 * Q);
    cr.close_path();
}

/// SVG path data: M L H V C S Q T Z, absolute and relative. Arcs (A) become lines.
pub fn path(cr: &cairo::Context, d: &str) {
    let mut toks: Vec<String> = vec![];
    let mut cur = String::new();
    for c in d.chars() {
        if c.is_ascii_alphabetic() && c != 'e' && c != 'E' {
            if !cur.is_empty() {
                toks.push(std::mem::take(&mut cur));
            }
            toks.push(c.to_string());
        } else if c == ',' || c.is_whitespace() {
            if !cur.is_empty() {
                toks.push(std::mem::take(&mut cur));
            }
        } else if (c == '-' && !cur.is_empty() && !cur.ends_with(['e', 'E'])) || (c == '.' && cur.contains('.')) {
            // A sign or a second point starts the next number ("1-2", "0.5.5").
            toks.push(std::mem::take(&mut cur));
            cur.push(c);
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    let (mut x, mut y) = (0.0f64, 0.0f64);
    let (mut sx, mut sy) = (0.0, 0.0);
    let mut last_ctrl: Option<(f64, f64)> = None;
    let mut cmd = 'M';
    let mut i = 0;
    let num = |i: &mut usize| -> Option<f64> {
        let v = toks.get(*i)?.parse::<f64>().ok()?;
        *i += 1;
        Some(v)
    };
    while i < toks.len() {
        if let Some(c) = toks[i].chars().next().filter(|c| c.is_ascii_alphabetic()) {
            cmd = c;
            i += 1;
            if matches!(cmd, 'Z' | 'z') {
                cr.close_path();
                x = sx;
                y = sy;
                continue;
            }
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };
        let before = i;
        match cmd.to_ascii_uppercase() {
            'M' => {
                let (Some(a), Some(b)) = (num(&mut i), num(&mut i)) else { break };
                x = ox + a;
                y = oy + b;
                cr.move_to(x, y);
                sx = x;
                sy = y;
                cmd = if rel { 'l' } else { 'L' };
                last_ctrl = None;
            }
            'L' => {
                let (Some(a), Some(b)) = (num(&mut i), num(&mut i)) else { break };
                x = ox + a;
                y = oy + b;
                cr.line_to(x, y);
                last_ctrl = None;
            }
            'H' => {
                let Some(a) = num(&mut i) else { break };
                x = ox + a;
                cr.line_to(x, y);
            }
            'V' => {
                let Some(b) = num(&mut i) else { break };
                y = oy + b;
                cr.line_to(x, y);
            }
            'C' => {
                let v: Vec<f64> = (0..6).filter_map(|_| num(&mut i)).collect();
                if v.len() < 6 {
                    break;
                }
                cr.curve_to(ox + v[0], oy + v[1], ox + v[2], oy + v[3], ox + v[4], oy + v[5]);
                last_ctrl = Some((ox + v[2], oy + v[3]));
                x = ox + v[4];
                y = oy + v[5];
            }
            'S' => {
                let v: Vec<f64> = (0..4).filter_map(|_| num(&mut i)).collect();
                if v.len() < 4 {
                    break;
                }
                let (c1x, c1y) = last_ctrl.map_or((x, y), |(cx, cy)| (2.0 * x - cx, 2.0 * y - cy));
                cr.curve_to(c1x, c1y, ox + v[0], oy + v[1], ox + v[2], oy + v[3]);
                last_ctrl = Some((ox + v[0], oy + v[1]));
                x = ox + v[2];
                y = oy + v[3];
            }
            'Q' | 'T' => {
                let smooth = cmd.eq_ignore_ascii_case(&'T');
                let n = if smooth { 2 } else { 4 };
                let v: Vec<f64> = (0..n).filter_map(|_| num(&mut i)).collect();
                if v.len() < n {
                    break;
                }
                let (qx, qy) = if smooth { last_ctrl.map_or((x, y), |(cx, cy)| (2.0 * x - cx, 2.0 * y - cy)) } else { (ox + v[0], oy + v[1]) };
                let (ex, ey) = (ox + v[n - 2], oy + v[n - 1]);
                cr.curve_to(x + 2.0 / 3.0 * (qx - x), y + 2.0 / 3.0 * (qy - y), ex + 2.0 / 3.0 * (qx - ex), ey + 2.0 / 3.0 * (qy - ey), ex, ey);
                last_ctrl = Some((qx, qy));
                x = ex;
                y = ey;
            }
            'A' => {
                let v: Vec<f64> = (0..7).filter_map(|_| num(&mut i)).collect();
                if v.len() < 7 {
                    break;
                }
                x = ox + v[5];
                y = oy + v[6];
                cr.line_to(x, y);
            }
            _ => break,
        }
        if i == before {
            break;
        }
    }
}

#[repr(C)]
struct RsvgRectangle {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[link(name = "rsvg-2")]
unsafe extern "C" {
    fn rsvg_handle_new_from_data(data: *const u8, len: usize, error: *mut *mut gtk::glib::ffi::GError) -> *mut std::ffi::c_void;
    fn rsvg_handle_render_document(
        handle: *mut std::ffi::c_void,
        cr: *mut gtk::cairo::ffi::cairo_t,
        viewport: *const RsvgRectangle,
        error: *mut *mut gtk::glib::ffi::GError,
    ) -> gtk::glib::ffi::gboolean;
}

/// Draw an SVG document scaled into `w × h`.
pub fn draw_svg(cr: &cairo::Context, svg: &str, w: f64, h: f64) {
    unsafe {
        let mut err = std::ptr::null_mut();
        let handle = rsvg_handle_new_from_data(svg.as_ptr(), svg.len(), &mut err);
        if handle.is_null() {
            if !err.is_null() {
                gtk::glib::ffi::g_error_free(err);
            }
            return;
        }
        let viewport = RsvgRectangle { x: 0.0, y: 0.0, width: w, height: h };
        let mut err = std::ptr::null_mut();
        rsvg_handle_render_document(handle, cr.to_raw_none(), &viewport, &mut err);
        if !err.is_null() {
            gtk::glib::ffi::g_error_free(err);
        }
        gtk::glib::gobject_ffi::g_object_unref(handle as *mut _);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface() -> (cairo::ImageSurface, cairo::Context) {
        let s = cairo::ImageSurface::create(cairo::Format::ARgb32, 24, 24).unwrap();
        let cr = cairo::Context::new(&s).unwrap();
        (s, cr)
    }

    fn painted(s: &mut cairo::ImageSurface) -> usize {
        s.flush();
        s.data().unwrap().chunks(4).filter(|p| p[3] != 0).count()
    }

    #[test]
    fn draws_commands_and_svg() {
        let (mut s, cr) = surface();
        let v = Vector {
            commands: vec![Draw::Path { d: "M2 2 h20 v20 H2 z".into(), fill: Some(Paint::Color("#ff0000".into())), stroke: None, width: None }],
            view_box: Some([0.0, 0.0, 24.0, 24.0]),
            ..Default::default()
        };
        draw(&cr, &v, 24.0, 24.0);
        drop(cr);
        assert!(painted(&mut s) > 300);

        let (mut s, cr) = surface();
        draw_svg(&cr, r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><circle cx="5" cy="5" r="5" fill="#00f"/></svg>"##, 24.0, 24.0);
        drop(cr);
        assert!(painted(&mut s) > 300);
    }
}
