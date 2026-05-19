//! GPUI-based painter for the editor canvas.
//!
//! The toolbar lives in `platform::driver` (composed from regular GPUI
//! elements). This module is the low-level half: free functions that turn
//! the canvas state into `paint_path` / `paint_quad` calls on a [`Window`].
//! It carries no state of its own.

use gpui::{
    App, Background, Bounds, FontWeight, Hsla, PathBuilder, PathStyle, Pixels, Point,
    StrokeOptions, TextAlign, TextRun, Window, hsla, point, px, quad, size, transparent_black,
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

const HINT_BG: Hsla = Hsla {
    h: 0.66,
    s: 0.10,
    l: 0.10,
    a: 0.92,
};

const HINT_FG: Hsla = Hsla {
    h: 0.0,
    s: 0.0,
    l: 0.94,
    a: 1.0,
};

/// Transform from canvas-space (physical px, global) to window-space
/// (logical px, monitor-local). `inv_scale = 1.0 / window.scale_factor()`.
#[derive(Clone, Copy)]
pub struct Xform {
    pub origin: (i32, i32),
    pub inv_scale: f32,
}

impl Xform {
    pub fn new(origin: (i32, i32), scale: f32) -> Self {
        Self {
            origin,
            inv_scale: if scale > 0.0 { 1.0 / scale } else { 1.0 },
        }
    }

    fn pt(&self, p: FPoint) -> Point<Pixels> {
        point(
            px((p.x - self.origin.0 as f32) * self.inv_scale),
            px((p.y - self.origin.1 as f32) * self.inv_scale),
        )
    }

    fn len(&self, v: f32) -> f32 {
        v * self.inv_scale
    }

    fn rect(&self, r: CapRect) -> Bounds<Pixels> {
        Bounds {
            origin: point(
                px((r.x() - self.origin.0) as f32 * self.inv_scale),
                px((r.y() - self.origin.1) as f32 * self.inv_scale),
            ),
            size: size(
                px(r.width() as f32 * self.inv_scale),
                px(r.height() as f32 * self.inv_scale),
            ),
        }
    }
}

/// Paint the region rubber-band and every shape onto `window`.
pub fn paint_canvas(window: &mut Window, _cx: &mut App, canvas: &Canvas, xf: Xform) {
    if let Some(rect) = canvas.region() {
        paint_rect_stroke(window, rect, xf, 2.0, ACCENT);
    }
    for shape in canvas.shapes() {
        paint_shape(window, shape, xf);
    }
    if let Some(preview) = canvas.preview_shape() {
        paint_shape(window, &preview, xf);
    }
    if let Some(pending) = canvas.pending_text() {
        paint_shape(window, &pending, xf);
    }
    if let Some(verts) = canvas.polygon_vertices() {
        if verts.len() >= 2 {
            let style = canvas.current_polygon_style();
            let pts: Vec<FPoint> = verts.to_vec();
            paint_polyline(
                window,
                &pts,
                xf,
                style.stroke_width.max(1.0),
                color_to_hsla(style.stroke),
                false,
            );
        }
    }
}

fn paint_shape(window: &mut Window, shape: &Shape, xf: Xform) {
    let stroke = color_to_hsla(shape.style.stroke);
    let width = shape.style.stroke_width.max(1.0);
    let fill = shape.style.fill.map(color_to_hsla);

    match &shape.kind {
        ShapeKind::FreehandStroke { points } => {
            paint_polyline(window, points, xf, width, stroke, false);
        }
        ShapeKind::Line { from, to } => {
            paint_polyline(window, &[*from, *to], xf, width, stroke, false);
        }
        ShapeKind::Arrow { from, to } => {
            paint_polyline(window, &[*from, *to], xf, width, stroke, false);
            let a = xf.pt(*from);
            let b = xf.pt(*to);
            let dx = (b.x - a.x).as_f32();
            let dy = (b.y - a.y).as_f32();
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let ux = dx / len;
            let uy = dy / len;
            let head = xf.len((width * 3.0).max(10.0));
            let p1 = point(
                px(b.x.as_f32() - (ux * head + uy * head * 0.5)),
                px(b.y.as_f32() - (uy * head - ux * head * 0.5)),
            );
            let p2 = point(
                px(b.x.as_f32() - (ux * head - uy * head * 0.5)),
                px(b.y.as_f32() - (uy * head + ux * head * 0.5)),
            );
            stroke_segments(window, &[(b, p1), (b, p2)], xf.len(width), stroke);
        }
        ShapeKind::Rectangle { rect } => {
            let r = xf.rect(*rect);
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
            paint_rect_stroke(window, *rect, xf, width, stroke);
        }
        ShapeKind::BlurRect { rect, .. } => {
            paint_rect_stroke(window, *rect, xf, width.max(1.5), stroke);
        }
        ShapeKind::Ellipse { rect } => {
            let r = xf.rect(*rect);
            let cx = r.origin.x + r.size.width / 2.;
            let cy = r.origin.y + r.size.height / 2.;
            let rx = r.size.width / 2.;
            let ry = r.size.height / 2.;
            paint_ellipse(window, point(cx, cy), rx, ry, xf.len(width), stroke, fill);
        }
        ShapeKind::Step {
            center, radius, ..
        } => {
            let c = xf.pt(*center);
            let r_logical = px(xf.len(*radius));
            paint_ellipse(
                window,
                c,
                r_logical,
                r_logical,
                1.0,
                hsla(0.0, 0.0, 1.0, 1.0),
                fill.or(Some(stroke)),
            );
        }
        ShapeKind::Text { .. } => {
            // Live text is rendered via `paint_step_label` / `paint_text` in
            // a separate pass that has access to the text system. See the
            // companion helpers in `platform::driver`.
        }
        ShapeKind::Polygon { points, closed } => {
            if points.is_empty() {
                return;
            }
            paint_polyline(window, points, xf, width, stroke, *closed);
        }
    }
}

fn paint_polyline(
    window: &mut Window,
    points: &[FPoint],
    xf: Xform,
    width: f32,
    color: Hsla,
    closed: bool,
) {
    if points.len() < 2 {
        return;
    }
    let mut builder = PathBuilder::stroke(px(xf.len(width)));
    let first = xf.pt(points[0]);
    builder.move_to(first);
    for p in &points[1..] {
        builder.line_to(xf.pt(*p));
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
    xf: Xform,
    width: f32,
    color: Hsla,
) {
    let r = xf.rect(rect);
    let tl = r.origin;
    let tr = point(tl.x + r.size.width, tl.y);
    let br = point(tl.x + r.size.width, tl.y + r.size.height);
    let bl = point(tl.x, tl.y + r.size.height);
    let mut builder = PathBuilder::stroke(px(xf.len(width)));
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

/// Paint the "Press Enter to accept" pill underneath the active region.
///
/// `monitor_bounds` and `region` are in canvas-space (physical px). Only
/// the monitor whose horizontal centre line covers the region draws the
/// hint, so multi-monitor selections don't show duplicates.
pub fn paint_confirm_hint(
    window: &mut Window,
    cx: &mut App,
    monitor_bounds: CapRect,
    region: Option<CapRect>,
    xf: Xform,
) {
    let text = "Press Enter to accept";
    let font_size = px(14.);
    let mut font = window.text_style().font();
    font.weight = FontWeight::SEMIBOLD;
    let runs = [TextRun {
        len: text.len(),
        font,
        color: HINT_FG,
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    let line =
        window
            .text_system()
            .shape_line(text.into(), font_size, &runs, None);
    let text_w = line.width().as_f32();
    let line_h = window.line_height().as_f32().max(font_size.as_f32() * 1.3);
    let pad_x = 14.0_f32;
    let pad_y = 6.0_f32;
    let panel_w = text_w + pad_x * 2.0;
    let panel_h = line_h + pad_y * 2.0;

    let mb = xf.rect(monitor_bounds);
    let mw = mb.size.width.as_f32();
    let mh = mb.size.height.as_f32();

    let (panel_x, panel_y) = match region.filter(|r| r.width() >= 2 && r.height() >= 2) {
        Some(r) => {
            let rl = xf.rect(r);
            let cx_local = (rl.origin.x + rl.size.width / 2.0).as_f32();
            if cx_local < 0.0 || cx_local >= mw {
                return;
            }
            let region_top = rl.origin.y.as_f32();
            let region_bottom = region_top + rl.size.height.as_f32();
            let margin = 16.0;
            let below = region_bottom + margin;
            let panel_y = if below + panel_h <= mh - 8.0 {
                below
            } else if region_top - panel_h - margin >= 8.0 {
                region_top - panel_h - margin
            } else {
                (mh - panel_h - 8.0).max(8.0)
            };
            let max_x = (mw - panel_w - 8.0).max(8.0);
            let panel_x = (cx_local - panel_w / 2.0).clamp(8.0, max_x);
            (panel_x, panel_y)
        }
        None => (
            (mw - panel_w) / 2.0,
            (mh - panel_h - 48.0).max(8.0),
        ),
    };

    let origin = point(px(panel_x), px(panel_y));
    let panel_bounds = Bounds {
        origin,
        size: size(px(panel_w), px(panel_h)),
    };
    window.paint_quad(quad(
        panel_bounds,
        px(6.),
        Background::from(HINT_BG),
        px(1.),
        ACCENT,
        Default::default(),
    ));
    let text_origin = point(px(panel_x + pad_x), px(panel_y + pad_y));
    let _ = line.paint(
        text_origin,
        px(line_h),
        TextAlign::Left,
        None,
        window,
        cx,
    );
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

