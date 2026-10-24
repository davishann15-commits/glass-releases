//! The audio visualizer under the lyrics: bars, mirrored bars or a wave, in
//! a gradient between `[visualizer]` color and color2 (the accent and the
//! foreground by default), so it follows the theme. It shows audio played on
//! this computer; while another device plays, it rests quietly.

use super::theme::{self, col, px};
use crate::app::App;
use eframe::egui::{self, Color32, Mesh, Pos2, Rect, Shape, Stroke, Ui, pos2};

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

fn faded(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}

/// A rectangle filled bottom-to-top from `low` to `high`.
fn gradient_rect(mesh: &mut Mesh, r: Rect, low: Color32, high: Color32) {
    let i = mesh.vertices.len() as u32;
    mesh.colored_vertex(r.left_bottom(), low);
    mesh.colored_vertex(r.right_bottom(), low);
    mesh.colored_vertex(r.right_top(), high);
    mesh.colored_vertex(r.left_top(), high);
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
}

/// Points on a smooth curve through `pts` (Catmull-Rom).
fn smooth(pts: &[Pos2], steps: usize) -> Vec<Pos2> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = Vec::with_capacity(pts.len() * steps);
    for i in 0..pts.len() - 1 {
        let p0 = pts[i.saturating_sub(1)];
        let (p1, p2) = (pts[i], pts[i + 1]);
        let p3 = pts[(i + 2).min(pts.len() - 1)];
        for s in 0..steps {
            let t = s as f32 / steps as f32;
            let (t2, t3) = (t * t, t * t * t);
            let f = |a: f32, b: f32, c: f32, d: f32| 0.5 * ((2.0 * b) + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3);
            out.push(pos2(f(p0.x, p1.x, p2.x, p3.x), f(p0.y, p1.y, p2.y, p3.y)));
        }
    }
    out.push(*pts.last().unwrap());
    out
}

pub fn show(app: &mut App, ui: &mut Ui, rect: Rect) {
    let st = theme::viz_style();
    let n = st.bars.clamp(12, 128) as usize;
    // Calmer when the system asks for less motion.
    let smoothing = if theme::reduce_motion() { st.smoothing.max(0.85) } else { st.smoothing };
    app.viz.update(n, st.sensitivity, smoothing);
    let painter = ui.painter().with_clip_rect(rect.expand(px(6.0)));
    let (low, high) = (faded(st.color.color32(), st.opacity), faded(st.color2.color32(), st.opacity));
    let (bands, peaks) = (&app.viz.bands, &app.viz.peaks);
    let slot = rect.width() / n as f32;
    let bw = (slot * 0.62).max(px(1.5));
    let mut mesh = Mesh::default();
    // Drawn last, on top: caps and peaks.
    let mut extra: Vec<Shape> = Vec::new();
    match st.style.as_str() {
        "mirror" => {
            let mid = rect.center().y;
            let half = rect.height() / 2.0;
            for (i, v) in bands.iter().enumerate() {
                let x = rect.left() + slot * (i as f32 + 0.5);
                let h = (v * half).max(px(1.0));
                let top = lerp(low, high, *v);
                gradient_rect(&mut mesh, Rect::from_min_max(pos2(x - bw / 2.0, mid - h), pos2(x + bw / 2.0, mid)), low, top);
                // The reflection below, fainter.
                gradient_rect(&mut mesh, Rect::from_min_max(pos2(x - bw / 2.0, mid), pos2(x + bw / 2.0, mid + h * 0.7)), faded(top, 0.35), faded(low, 0.05));
                if st.glow && *v > 0.05 {
                    for (k, a) in [(2.4f32, 0.05f32), (1.5, 0.09)] {
                        extra.push(Shape::circle_filled(pos2(x, mid - h), bw * k * 0.5, faded(top, a * (0.4 + v))));
                    }
                }
            }
        }
        "wave" => {
            let base = rect.bottom();
            let pts: Vec<Pos2> = bands.iter().enumerate().map(|(i, v)| pos2(rect.left() + slot * (i as f32 + 0.5), base - v * rect.height() * 0.92)).collect();
            // Smooth, but never below the baseline.
            let curve: Vec<Pos2> = smooth(&pts, 6).into_iter().map(|p| pos2(p.x, p.y.min(base))).collect();
            // Fill under the curve, fading to nothing at the bottom.
            for w in curve.windows(2) {
                let (a, b) = (w[0], w[1]);
                let lvl = ((base - a.y) / rect.height()).clamp(0.0, 1.0);
                let i = mesh.vertices.len() as u32;
                mesh.colored_vertex(pos2(a.x, base), faded(low, 0.05));
                mesh.colored_vertex(pos2(b.x, base), faded(low, 0.05));
                mesh.colored_vertex(b, faded(lerp(low, high, lvl), 0.55));
                mesh.colored_vertex(a, faded(lerp(low, high, lvl), 0.55));
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
            }
            if st.glow {
                painter.add(Shape::line(curve.clone(), Stroke::new(px(6.0), faded(high, 0.12))));
            }
            painter.add(Shape::mesh(mesh));
            painter.add(Shape::line(curve, Stroke::new(px(2.0), high)));
            mesh = Mesh::default();
        }
        _ => {
            let base = rect.bottom();
            for (i, v) in bands.iter().enumerate() {
                let x = rect.left() + slot * (i as f32 + 0.5);
                let h = (v * rect.height()).max(px(2.0));
                let top = lerp(low, high, *v);
                let r = Rect::from_min_max(pos2(x - bw / 2.0, base - h), pos2(x + bw / 2.0, base));
                gradient_rect(&mut mesh, r, low, top);
                if st.glow && *v > 0.05 {
                    // A soft halo at the tip, brighter the louder the band.
                    for (k, a) in [(2.6f32, 0.05f32), (1.7, 0.08), (1.15, 0.12)] {
                        extra.push(Shape::circle_filled(pos2(x, base - h), bw * k * 0.5, faded(top, a * (0.4 + v))));
                    }
                }
                // A rounded cap, and the falling peak above it.
                if *v > 0.02 {
                    extra.push(Shape::circle_filled(pos2(x, base - h), bw / 2.0, top));
                }
                let py = base - peaks[i] * rect.height() - px(4.0);
                if peaks[i] > 0.04 {
                    extra.push(Shape::line_segment([pos2(x - bw / 2.0, py), pos2(x + bw / 2.0, py)], Stroke::new(px(1.5), faded(high, 0.8))));
                }
            }
        }
    }
    if !mesh.vertices.is_empty() {
        painter.add(Shape::mesh(mesh));
    }
    painter.extend(extra);
    if !app.viz.live && !app.viz.moving() {
        // Resting: a thin line, and a hint when the music plays elsewhere.
        let y = if st.style == "mirror" { rect.center().y } else { rect.bottom() - px(1.0) };
        painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(1.0, faded(low, 0.35)));
        let elsewhere = app.pb.is_playing && !app.is_local() && !app.local_file_active();
        if elsewhere {
            // Mirror rests on the middle line, so the hint sits just above it rather than on it.
            let (at, align) = if st.style == "mirror" { (pos2(rect.center().x, y - px(10.0)), egui::Align2::CENTER_BOTTOM) } else { (rect.center(), egui::Align2::CENTER_CENTER) };
            painter.text(at, align, "the visualizer follows music played on this computer", theme::regular(11.5), col::muted());
        }
    }
    if app.viz.moving() {
        // 60 fps while Glass is in front; 30 behind other windows or with Reduce Motion
        // (each frame shown costs about as much to present as to draw). A hidden window
        // isn't drawn at all.
        let smooth = ui.input(|i| i.focused) && !theme::reduce_motion();
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(if smooth { 16 } else { 33 }));
    }
}
