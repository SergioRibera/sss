//! GPUI-based painter for the editor canvas.
//!
//! The toolbar lives in `platform::driver` (composed from regular GPUI
//! elements). This module is the low-level half: free functions that turn
//! the canvas state into `paint_path` / `paint_quad` calls on a [`Window`].
//! It carries no state of its own.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    App, Background, Bounds, Corners, FontWeight, Hsla, PathBuilder, PathStyle, Pixels, Point,
    RenderImage, StrokeOptions, TextAlign, TextRun, Window, hsla, point, px, quad, size,
    transparent_black,
};
use image::{Frame, ImageBuffer, Rgba};
use sss_capture::{Image as CapImage, Rect as CapRect};

use crate::canvas::Canvas;
use crate::color::Color;
use crate::geometry::FPoint;
use crate::shape::{Shape, ShapeId, ShapeKind};

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

impl Default for Xform {
    /// Identity transform: no origin offset, no scale.
    fn default() -> Self {
        Self {
            origin: (0, 0),
            inv_scale: 1.0,
        }
    }
}

impl Xform {
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

/// Cache of CPU-blurred image tiles keyed by shape geometry, so the
/// expensive `gaussian_blur` only runs when the user actually changes
/// the rectangle / radius. Lives on the per-window `OverlayView`.
#[derive(Default)]
pub struct BlurCache {
    entries: HashMap<BlurKey, Arc<RenderImage>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct BlurKey {
    shape: Option<ShapeId>,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    radius_thousandths: i32,
}

impl BlurCache {
    fn key(shape: Option<ShapeId>, rect: CapRect, radius: f32) -> BlurKey {
        BlurKey {
            shape,
            x: rect.x(),
            y: rect.y(),
            w: rect.width(),
            h: rect.height(),
            radius_thousandths: (radius * 1000.0).round() as i32,
        }
    }

    /// Evict everything except the entries still referenced by a current
    /// shape. Called once per paint pass so deleted blurs are cleaned up.
    fn retain_used(&mut self, used: &std::collections::HashSet<BlurKey>) {
        self.entries.retain(|k, _| used.contains(k));
    }
}

/// Paint the blurred underlay for every `BlurRect` shape (committed and
/// preview), using `initial` as the source frame. Call this **before**
/// `paint_canvas` so the rubber-band / outline draws on top.
pub fn paint_blurs(
    window: &mut Window,
    canvas: &Canvas,
    initial: Option<&CapImage>,
    initial_origin: (i32, i32),
    xf: Xform,
    cache: &mut BlurCache,
) {
    let Some(initial) = initial else { return };
    let buffer = initial.as_rgba();
    let (iw, ih) = buffer.dimensions();
    let mut used = std::collections::HashSet::new();

    let paint_one = |window: &mut Window,
                         cache: &mut BlurCache,
                         used: &mut std::collections::HashSet<BlurKey>,
                         shape_id: Option<ShapeId>,
                         rect: CapRect,
                         radius: f32| {
        let key = BlurCache::key(shape_id, rect, radius);
        used.insert(key);
        let image = cache
            .entries
            .entry(key)
            .or_insert_with(|| build_blurred(buffer, initial_origin, rect, radius, iw, ih));
        let bounds = xf.rect(rect);
        let _ = window.paint_image(bounds, Corners::all(px(0.)), image.clone(), 0, false);
    };

    for shape in canvas.shapes() {
        if let ShapeKind::BlurRect { rect, radius } = &shape.kind {
            paint_one(window, cache, &mut used, Some(shape.id), *rect, *radius);
        }
    }
    if let Some(preview) = canvas.preview_shape() {
        if let ShapeKind::BlurRect { rect, radius } = &preview.kind {
            paint_one(window, cache, &mut used, None, *rect, *radius);
        }
    }

    cache.retain_used(&used);
}

fn build_blurred(
    source: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    source_origin: (i32, i32),
    rect: CapRect,
    radius: f32,
    iw: u32,
    ih: u32,
) -> Arc<RenderImage> {
    // Intersect with the source buffer; if the rect spills outside (e.g.
    // off-screen during a drag) we clip rather than panic.
    let x0 = (rect.x() - source_origin.0).max(0) as u32;
    let y0 = (rect.y() - source_origin.1).max(0) as u32;
    let w = rect.width().min(iw.saturating_sub(x0)).max(1);
    let h = rect.height().min(ih.saturating_sub(y0)).max(1);
    let cropped = image::imageops::crop_imm(source, x0, y0, w, h).to_image();
    let mut blurred = sss_core::blur::gaussian_blur(cropped, radius.max(1.0));
    // GPUI sprites use BGRA premultiplied; the source is RGBA premultiplied.
    for px in blurred.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let frame = Frame::new(blurred);
    Arc::new(RenderImage::new(vec![frame]))
}

/// Paint the region rubber-band and every shape onto `window`.
pub fn paint_canvas(window: &mut Window, cx: &mut App, canvas: &Canvas, xf: Xform) {
    if let Some(rect) = canvas.region() {
        paint_rect_stroke(window, rect, xf, 2.0, ACCENT);
    }
    for shape in canvas.shapes() {
        paint_shape(window, cx, shape, xf, false);
    }
    if let Some(preview) = canvas.preview_shape() {
        paint_shape(window, cx, &preview, xf, false);
    }
    if let Some(pending) = canvas.pending_text() {
        // Editing-in-progress text: draw a blinking caret hint at the end.
        paint_shape(window, cx, &pending, xf, true);
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

fn paint_shape(window: &mut Window, cx: &mut App, shape: &Shape, xf: Xform, editing: bool) {
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
            center,
            number,
            radius,
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
            // Step number, centered. Matches composite.rs which uses
            // `radius * 1.1` to size the label.
            let label = number.to_string();
            let font_size = px(xf.len(*radius * 1.1));
            paint_centered_label(
                window,
                cx,
                &label,
                c,
                font_size,
                hsla(0.0, 0.0, 1.0, 1.0),
                false,
            );
        }
        ShapeKind::Text {
            origin: text_origin,
            content,
            style,
        } => {
            let color = color_to_hsla(style.color);
            let font_size = px(xf.len(style.size));
            let origin = xf.pt(*text_origin);
            let line_h = paint_text_run(
                window,
                cx,
                content,
                origin,
                font_size,
                color,
                style.bold,
            );
            if editing {
                // Vertical caret bar right after the typed string, same
                // height as the line. Compositors will repaint each frame
                // so a future "blink" is just a `cx.spawn` away — not
                // wired in this pass.
                let advance = measure_text_advance(window, content, font_size, style.bold);
                let caret_top = origin;
                let caret_x = origin.x + advance;
                let mut builder = PathBuilder::stroke(px(1.5_f32 * xf.inv_scale));
                builder.move_to(point(caret_x, caret_top.y));
                builder.line_to(point(caret_x, caret_top.y + line_h));
                if let Ok(path) = builder.build() {
                    window.paint_path(path, color);
                }
            }
        }
        ShapeKind::Polygon { points, closed } => {
            if points.is_empty() {
                return;
            }
            if *closed && points.len() >= 3 {
                if let Some(f) = fill {
                    let mut filled = PathBuilder::fill();
                    filled.move_to(xf.pt(points[0]));
                    for p in &points[1..] {
                        filled.line_to(xf.pt(*p));
                    }
                    filled.close();
                    if let Ok(path) = filled.build() {
                        window.paint_path(path, f);
                    }
                }
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

fn paint_text_run(
    window: &mut Window,
    cx: &mut App,
    text: &str,
    origin: Point<Pixels>,
    font_size: Pixels,
    color: Hsla,
    bold: bool,
) -> Pixels {
    if text.is_empty() {
        return font_size * 1.3;
    }
    let mut font = window.text_style().font();
    if bold {
        font.weight = FontWeight::BOLD;
    }
    let runs = [TextRun {
        len: text.len(),
        font,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    let line = window
        .text_system()
        .shape_line(text.into(), font_size, &runs, None);
    let line_h = px(font_size.as_f32() * 1.3);
    let _ = line.paint(origin, line_h, TextAlign::Left, None, window, cx);
    line_h
}

fn paint_centered_label(
    window: &mut Window,
    cx: &mut App,
    text: &str,
    center: Point<Pixels>,
    font_size: Pixels,
    color: Hsla,
    bold: bool,
) {
    if text.is_empty() {
        return;
    }
    let mut font = window.text_style().font();
    if bold {
        font.weight = FontWeight::BOLD;
    }
    let runs = [TextRun {
        len: text.len(),
        font,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    let line = window
        .text_system()
        .shape_line(text.into(), font_size, &runs, None);
    let line_h = px(font_size.as_f32() * 1.3);
    let origin = point(
        center.x - line.width() / 2.,
        center.y - line_h / 2.,
    );
    let _ = line.paint(origin, line_h, TextAlign::Left, None, window, cx);
}

fn measure_text_advance(
    window: &mut Window,
    text: &str,
    font_size: Pixels,
    bold: bool,
) -> Pixels {
    if text.is_empty() {
        return px(0.);
    }
    let mut font = window.text_style().font();
    if bold {
        font.weight = FontWeight::BOLD;
    }
    let runs = [TextRun {
        len: text.len(),
        font,
        color: hsla(0., 0., 0., 1.),
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    window
        .text_system()
        .shape_line(text.into(), font_size, &runs, None)
        .width()
}

/// Paint a hover highlight over a rect for Monitor/Window selector modes.
/// `label` is rendered centered inside the rect (clipped to the monitor's
/// viewport so multi-output overlays don't draw duplicates).
pub fn paint_hover_target(
    window: &mut Window,
    cx: &mut App,
    target: CapRect,
    label: Option<&str>,
    monitor_bounds: CapRect,
    xf: Xform,
) {
    // Only the monitor that contains the target's centre draws it, so a
    // window that straddles two outputs doesn't double-highlight.
    let cx_g = target.x() + target.width() as i32 / 2;
    let cy_g = target.y() + target.height() as i32 / 2;
    if cx_g < monitor_bounds.x()
        || cx_g >= monitor_bounds.x() + monitor_bounds.width() as i32
        || cy_g < monitor_bounds.y()
        || cy_g >= monitor_bounds.y() + monitor_bounds.height() as i32
    {
        return;
    }
    let r = xf.rect(target);
    // Semi-transparent fill so the underlying screenshot stays visible.
    window.paint_quad(quad(
        r,
        px(0.),
        Background::from(hsla(0.58, 0.7, 0.55, 0.18)),
        px(0.),
        transparent_black(),
        Default::default(),
    ));
    paint_rect_stroke(window, target, xf, 3.0, ACCENT);

    let Some(text) = label.filter(|t| !t.is_empty()) else {
        return;
    };
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
    let line = window
        .text_system()
        .shape_line(text.into(), font_size, &runs, None);
    let text_w = line.width().as_f32();
    let line_h = window.line_height().as_f32().max(font_size.as_f32() * 1.3);
    let pad_x = 12.0;
    let pad_y = 5.0;
    let panel_w = text_w + pad_x * 2.0;
    let panel_h = line_h + pad_y * 2.0;
    let center_local_x = (r.origin.x + r.size.width / 2.0).as_f32();
    let center_local_y = (r.origin.y + r.size.height / 2.0).as_f32();
    let panel_x = center_local_x - panel_w / 2.0;
    let panel_y = center_local_y - panel_h / 2.0;
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

