//! GPUI-based painter for the editor canvas.
//!
//! The toolbar lives in `platform::driver` (composed from regular GPUI
//! elements). This module is the low-level half: free functions that turn
//! the canvas state into `paint_path` / `paint_quad` calls on a [`Window`].
//! It carries no state of its own.

use gpui::{
    App, Background, Bounds, Hsla, PathBuilder, PathStyle, Pixels, Point, StrokeOptions, Window,
    hsla, point, px, quad, size, transparent_black,
};
use sss_capture::Rect as CapRect;

use crate::canvas::Canvas;
use crate::color::Color;
use crate::geometry::FPoint;
use crate::shape::{Shape, ShapeKind};

/// Highlight blue used for the rubber-band.
const ACCENT: Hsla = Hsla {
    h: 0.58,
    s: 0.78,
    l: 0.66,
    a: 1.0,
};

/// Paint the region rubber-band and every shape onto `window`.
///
/// `origin` is the monitor's global top-left in canvas coordinates; we
/// subtract it from every point so the same canvas state renders correctly
/// on every per-output overlay.
pub fn paint_canvas(window: &mut Window, _cx: &mut App, canvas: &Canvas, origin: (i32, i32)) {
    if let Some(rect) = canvas.region() {
        paint_rect_stroke(window, rect, origin, 2.0, ACCENT);
    }
    for shape in canvas.shapes() {
        paint_shape(window, shape, origin);
    }
    if let Some(preview) = canvas.preview_shape() {
        paint_shape(window, &preview, origin);
    }
    if let Some(pending) = canvas.pending_text() {
        paint_shape(window, &pending, origin);
    }
    if let Some(verts) = canvas.polygon_vertices() {
        if verts.len() >= 2 {
            let style = canvas.current_polygon_style();
            let pts: Vec<FPoint> = verts.to_vec();
            paint_polyline(
                window,
                &pts,
                origin,
                style.stroke_width.max(1.0),
                color_to_hsla(style.stroke),
                false,
            );
        }
    }
}

fn paint_shape(window: &mut Window, shape: &Shape, origin: (i32, i32)) {
    let stroke = color_to_hsla(shape.style.stroke);
    let width = shape.style.stroke_width.max(1.0);
    let fill = shape.style.fill.map(color_to_hsla);

    match &shape.kind {
        ShapeKind::FreehandStroke { points } => {
            paint_polyline(window, points, origin, width, stroke, false);
        }
        ShapeKind::Line { from, to } => {
            paint_polyline(window, &[*from, *to], origin, width, stroke, false);
        }
        ShapeKind::Arrow { from, to } => {
            paint_polyline(window, &[*from, *to], origin, width, stroke, false);
            let a = local_pt(*from, origin);
            let b = local_pt(*to, origin);
            let dx = (b.x - a.x).as_f32();
            let dy = (b.y - a.y).as_f32();
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let ux = dx / len;
            let uy = dy / len;
            let head = (width * 3.0).max(10.0);
            let p1 = point(
                px(b.x.as_f32() - (ux * head + uy * head * 0.5)),
                px(b.y.as_f32() - (uy * head - ux * head * 0.5)),
            );
            let p2 = point(
                px(b.x.as_f32() - (ux * head - uy * head * 0.5)),
                px(b.y.as_f32() - (uy * head + ux * head * 0.5)),
            );
            stroke_segments(window, &[(b, p1), (b, p2)], width, stroke);
        }
        ShapeKind::Rectangle { rect } => {
            let r = cap_rect_local(*rect, origin);
            if let Some(f) = fill {
                window.paint_quad(quad(
                    r,
                    px(0.),
                    Background::from(f),
                    px(0.),
                    transparent_black(),
                    Default::default(),
                ));
            }
            paint_rect_stroke(window, *rect, origin, width, stroke);
        }
        ShapeKind::BlurRect { rect, .. } => {
            // GPU live blur preview is follow-up work; show the rectangle
            // outline so the user can see what region they've selected.
            paint_rect_stroke(window, *rect, origin, width.max(1.5), stroke);
        }
        ShapeKind::Ellipse { rect } => {
            let r = cap_rect_local(*rect, origin);
            let cx = r.origin.x + r.size.width / 2.;
            let cy = r.origin.y + r.size.height / 2.;
            let rx = r.size.width / 2.;
            let ry = r.size.height / 2.;
            paint_ellipse(window, point(cx, cy), rx, ry, width, stroke, fill);
        }
        ShapeKind::Step {
            center, radius, ..
        } => {
            // The numbered label is rendered by composite.rs when the
            // screenshot is baked. The interactive preview shows just the
            // circle; wiring shape_line into the canvas paint pass is
            // follow-up work.
            let c = local_pt(*center, origin);
            paint_ellipse(
                window,
                c,
                px(*radius),
                px(*radius),
                1.0,
                hsla(0.0, 0.0, 1.0, 1.0),
                fill.or(Some(stroke)),
            );
        }
        ShapeKind::Text { .. } => {
            // Same as Step: text is baked at export time by composite.rs.
        }
        ShapeKind::Polygon { points, closed } => {
            if points.is_empty() {
                return;
            }
            paint_polyline(window, points, origin, width, stroke, *closed);
        }
    }
}

fn paint_polyline(
    window: &mut Window,
    points: &[FPoint],
    origin: (i32, i32),
    width: f32,
    color: Hsla,
    closed: bool,
) {
    if points.len() < 2 {
        return;
    }
    let mut builder = PathBuilder::stroke(px(width));
    let first = local_pt(points[0], origin);
    builder.move_to(first);
    for p in &points[1..] {
        builder.line_to(local_pt(*p, origin));
    }
    if closed {
        builder.line_to(first);
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn stroke_segments(
    window: &mut Window,
    segments: &[(Point<Pixels>, Point<Pixels>)],
    width: f32,
    color: Hsla,
) {
    for (a, b) in segments {
        let mut builder = PathBuilder::stroke(px(width));
        builder.move_to(*a);
        builder.line_to(*b);
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
    }
}

fn paint_rect_stroke(
    window: &mut Window,
    rect: CapRect,
    origin: (i32, i32),
    width: f32,
    color: Hsla,
) {
    let r = cap_rect_local(rect, origin);
    let tl = r.origin;
    let tr = point(tl.x + r.size.width, tl.y);
    let br = point(tl.x + r.size.width, tl.y + r.size.height);
    let bl = point(tl.x, tl.y + r.size.height);
    let mut builder = PathBuilder::stroke(px(width));
    builder.move_to(tl);
    builder.line_to(tr);
    builder.line_to(br);
    builder.line_to(bl);
    builder.line_to(tl);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn paint_ellipse(
    window: &mut Window,
    center: Point<Pixels>,
    rx: Pixels,
    ry: Pixels,
    stroke_width: f32,
    stroke: Hsla,
    fill: Option<Hsla>,
) {
    let radii = point(rx, ry);
    let start = point(center.x + rx, center.y);
    let opposite = point(center.x - rx, center.y);

    if let Some(f) = fill {
        let mut filled = PathBuilder::fill();
        filled.move_to(start);
        filled.arc_to(radii, px(0.), false, false, opposite);
        filled.arc_to(radii, px(0.), false, false, start);
        filled.close();
        if let Ok(path) = filled.build() {
            window.paint_path(path, f);
        }
    }

    let mut outline = PathBuilder::stroke(px(stroke_width))
        .with_style(PathStyle::Stroke(StrokeOptions::default()));
    outline.move_to(start);
    outline.arc_to(radii, px(0.), false, false, opposite);
    outline.arc_to(radii, px(0.), false, false, start);
    if let Ok(path) = outline.build() {
        window.paint_path(path, stroke);
    }
}

fn local_pt(p: FPoint, origin: (i32, i32)) -> Point<Pixels> {
    point(px(p.x - origin.0 as f32), px(p.y - origin.1 as f32))
}

fn cap_rect_local(rect: CapRect, origin: (i32, i32)) -> Bounds<Pixels> {
    Bounds {
        origin: point(
            px((rect.x() - origin.0) as f32),
            px((rect.y() - origin.1) as f32),
        ),
        size: size(px(rect.width() as f32), px(rect.height() as f32)),
    }
}

fn color_to_hsla(c: Color) -> Hsla {
    let [r, g, b, a] = c.0;
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let max = rf.max(gf.max(bf));
    let min = rf.min(gf.min(bf));
    let l = (max + min) * 0.5;
    let (h, s) = if (max - min).abs() < f32::EPSILON {
        (0.0, 0.0)
    } else {
        let d = max - min;
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        let h = if max == rf {
            ((gf - bf) / d + if gf < bf { 6.0 } else { 0.0 }) / 6.0
        } else if max == gf {
            ((bf - rf) / d + 2.0) / 6.0
        } else {
            ((rf - gf) / d + 4.0) / 6.0
        };
        (h, s)
    };
    Hsla {
        h,
        s,
        l,
        a: a as f32 / 255.0,
    }
}
