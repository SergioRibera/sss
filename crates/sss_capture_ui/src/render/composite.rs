//! CPU rasteriser that bakes the canvas into the captured image.

use std::f32;

use image::imageops;
use image::{Rgba, RgbaImage};

use crate::canvas::Canvas;
use crate::color::Color;
use crate::geometry::FPoint;
use crate::shape::{Shape, ShapeKind, TextStyle};
use sss_capture::Rect;

/// Render every shape in `canvas` onto `image`; `origin` is the canvas
/// coordinate that the image's top-left corresponds to.
pub fn flatten(image: &mut RgbaImage, canvas: &Canvas, origin: (i32, i32)) {
    for shape in canvas.shapes() {
        paint_one(image, shape, origin);
    }
}

/// Render every shape that sits *below* the first `BlurRect` in z-order
/// — used to build the "below the blur" source for the live preview.
/// Shapes drawn on top of a blur don't contribute (they'd otherwise show
/// up in the blurred sample as ghost halos around the user's strokes).
pub fn flatten_below_first_blur(
    image: &mut RgbaImage,
    canvas: &Canvas,
    origin: (i32, i32),
) {
    for shape in canvas.shapes() {
        if matches!(shape.kind, ShapeKind::BlurRect { .. }) {
            break;
        }
        paint_one(image, shape, origin);
    }
}

/// Like [`flatten`] but also paints in-flight previews on top.
#[allow(dead_code)]
pub fn flatten_with_preview(image: &mut RgbaImage, canvas: &Canvas, origin: (i32, i32)) {
    for shape in canvas.shapes() {
        paint_one(image, shape, origin);
    }
    if let Some(preview) = canvas.preview_shape() {
        paint_one(image, &preview, origin);
    }
    if let Some(text) = canvas.pending_text() {
        paint_one(image, &text, origin);
    }
    if let Some(verts) = canvas.polygon_vertices() {
        if verts.len() >= 2 {
            let style = canvas.current_polygon_style();
            for pair in verts.windows(2) {
                stroke_line_aa(
                    image,
                    (
                        (pair[0].x - origin.0 as f32).round() as i32,
                        (pair[0].y - origin.1 as f32).round() as i32,
                    ),
                    (
                        (pair[1].x - origin.0 as f32).round() as i32,
                        (pair[1].y - origin.1 as f32).round() as i32,
                    ),
                    style.stroke,
                    style.stroke_width.max(1.0) as i32,
                );
            }
        }
        for v in verts {
            let cx = (v.x - origin.0 as f32).round() as i32;
            let cy = (v.y - origin.1 as f32).round() as i32;
            for dy in -2..=2 {
                for dx in -2..=2 {
                    px(image, cx + dx, cy + dy, Color::ACCENT);
                }
            }
        }
    }
}

fn paint_one(image: &mut RgbaImage, shape: &Shape, origin: (i32, i32)) {
    match &shape.kind {
        ShapeKind::BlurRect { rect, radius } => {
            apply_blur(image, *rect, *radius, origin);
        }
        _ => draw_shape(image, shape, origin),
    }
}

fn draw_shape(img: &mut RgbaImage, shape: &Shape, origin: (i32, i32)) {
    use ShapeKind::*;
    match &shape.kind {
        FreehandStroke { points } => {
            let color = shape.style.stroke;
            let w = shape.style.stroke_width.max(1.0) as i32;
            for pair in points.windows(2) {
                let a = local(pair[0], origin);
                let b = local(pair[1], origin);
                stroke_line_aa(img, a, b, color, w);
            }
        }
        Line { from, to } => {
            stroke_line_aa(
                img,
                local(*from, origin),
                local(*to, origin),
                shape.style.stroke,
                shape.style.stroke_width.max(1.0) as i32,
            );
        }
        Arrow { from, to } => {
            let a = local(*from, origin);
            let b = local(*to, origin);
            let w = shape.style.stroke_width.max(1.0) as i32;
            stroke_line_aa(img, a, b, shape.style.stroke, w);
            draw_arrowhead(img, a, b, shape.style.stroke, w);
        }
        Rectangle { rect } => {
            let r = local_rect(*rect, origin);
            if let Some(fill) = shape.style.fill {
                fill_rect(img, r, fill);
            }
            stroke_rect(
                img,
                r,
                shape.style.stroke,
                shape.style.stroke_width.max(1.0) as i32,
            );
        }
        Ellipse { rect } => {
            let r = local_rect(*rect, origin);
            stroke_ellipse(
                img,
                r,
                shape.style.stroke,
                shape.style.stroke_width.max(1.0) as i32,
                shape.style.fill,
            );
        }
        BlurRect { .. } => {}
        Step {
            center,
            number,
            radius,
        } => {
            let c = local(*center, origin);
            let r = *radius as i32;
            if r > 0 {
                fill_disk(img, c, r, shape.style.fill.unwrap_or(shape.style.stroke));
                draw_circle_outline(img, c, r, Color::WHITE, 1);
                draw_number_centered(img, c, *number, Color::WHITE, *radius);
            }
        }
        Text {
            origin: o,
            content,
            style,
        } => {
            draw_text(img, local(*o, origin), content, style);
        }
        Polygon { points, closed } => {
            if points.is_empty() {
                return;
            }
            let w = shape.style.stroke_width.max(1.0) as i32;
            if *closed {
                if let Some(fill) = shape.style.fill {
                    fill_polygon(img, points, origin, fill);
                }
            }
            for pair in points.windows(2) {
                stroke_line_aa(
                    img,
                    local(pair[0], origin),
                    local(pair[1], origin),
                    shape.style.stroke,
                    w,
                );
            }
            if *closed && points.len() >= 3 {
                let last = points[points.len() - 1];
                let first = points[0];
                stroke_line_aa(
                    img,
                    local(last, origin),
                    local(first, origin),
                    shape.style.stroke,
                    w,
                );
            }
        }
    }
}

fn fill_polygon(img: &mut RgbaImage, points: &[FPoint], origin: (i32, i32), color: Color) {
    if points.len() < 3 {
        return;
    }
    let pts: Vec<(f32, f32)> = points
        .iter()
        .map(|p| (p.x - origin.0 as f32, p.y - origin.1 as f32))
        .collect();
    let y_min = pts
        .iter()
        .map(|p| p.1.floor() as i32)
        .min()
        .unwrap_or(0)
        .max(0);
    let y_max = pts
        .iter()
        .map(|p| p.1.ceil() as i32)
        .max()
        .unwrap_or(0)
        .min(img.height() as i32 - 1);
    for y in y_min..=y_max {
        let yf = y as f32 + 0.5;
        let mut crossings: Vec<f32> = Vec::with_capacity(8);
        for i in 0..pts.len() {
            let (x0, y0) = pts[i];
            let (x1, y1) = pts[(i + 1) % pts.len()];
            if (y0 <= yf && y1 > yf) || (y1 <= yf && y0 > yf) {
                let t = (yf - y0) / (y1 - y0);
                crossings.push(x0 + t * (x1 - x0));
            }
        }
        crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for pair in crossings.chunks_exact(2) {
            let x0 = pair[0].ceil() as i32;
            let x1 = pair[1].floor() as i32;
            for x in x0..=x1 {
                px(img, x, y, color);
            }
        }
    }
}

fn local(p: FPoint, origin: (i32, i32)) -> (i32, i32) {
    (p.x.round() as i32 - origin.0, p.y.round() as i32 - origin.1)
}

fn local_rect(r: Rect, origin: (i32, i32)) -> Rect {
    Rect::from_xywh(r.x() - origin.0, r.y() - origin.1, r.width(), r.height())
}

fn px(img: &mut RgbaImage, x: i32, y: i32, c: Color) {
    if x < 0 || y < 0 {
        return;
    }
    let (w, h) = img.dimensions();
    if x as u32 >= w || y as u32 >= h {
        return;
    }
    let src = img.get_pixel(x as u32, y as u32).0;
    let blended = blend(src, c.0);
    img.put_pixel(x as u32, y as u32, Rgba(blended));
}

fn blend(dst: [u8; 4], src: [u8; 4]) -> [u8; 4] {
    let sa = src[3] as f32 / 255.0;
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a == 0.0 {
        return [0, 0, 0, 0];
    }
    let mix = |s: u8, d: u8| {
        let s = s as f32 / 255.0;
        let d = d as f32 / 255.0;
        (((s * sa + d * da * (1.0 - sa)) / out_a) * 255.0) as u8
    };
    [
        mix(src[0], dst[0]),
        mix(src[1], dst[1]),
        mix(src[2], dst[2]),
        (out_a * 255.0) as u8,
    ]
}

/// Anti-aliased thick line via distance-based coverage.
fn stroke_line_aa(img: &mut RgbaImage, a: (i32, i32), b: (i32, i32), c: Color, width: i32) {
    let af = (a.0 as f32, a.1 as f32);
    let bf = (b.0 as f32, b.1 as f32);
    let dx = bf.0 - af.0;
    let dy = bf.1 - af.1;
    let len2 = dx * dx + dy * dy;
    let half = (width as f32 / 2.0).max(0.5);
    let pad = half.ceil() as i32 + 1;
    let x_min = a.0.min(b.0) - pad;
    let y_min = a.1.min(b.1) - pad;
    let x_max = a.0.max(b.0) + pad;
    let y_max = a.1.max(b.1) + pad;
    for y in y_min..=y_max {
        for x in x_min..=x_max {
            let cx = x as f32 + 0.5;
            let cy = y as f32 + 0.5;
            let t = if len2 < 1.0e-6 {
                0.0
            } else {
                (((cx - af.0) * dx + (cy - af.1) * dy) / len2).clamp(0.0, 1.0)
            };
            let qx = af.0 + dx * t;
            let qy = af.1 + dy * t;
            let d = ((cx - qx).powi(2) + (cy - qy).powi(2)).sqrt();
            let coverage = if d <= half - 0.5 {
                1.0
            } else if d >= half + 0.5 {
                0.0
            } else {
                half + 0.5 - d
            };
            if coverage <= 0.0 {
                continue;
            }
            let alpha = (c.0[3] as f32 * coverage) as u8;
            let mut col = c;
            col.0[3] = alpha;
            px(img, x, y, col);
        }
    }
}

fn stroke_line(img: &mut RgbaImage, a: (i32, i32), b: (i32, i32), c: Color, width: i32) {
    let (mut x0, mut y0) = a;
    let (x1, y1) = b;
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let r = (width / 2).max(0);
    loop {
        fill_disk(img, (x0, y0), r, c);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

fn fill_disk(img: &mut RgbaImage, center: (i32, i32), r: i32, c: Color) {
    if r <= 0 {
        px(img, center.0, center.1, c);
        return;
    }
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r * r {
                px(img, center.0 + dx, center.1 + dy, c);
            }
        }
    }
}

fn fill_rect(img: &mut RgbaImage, r: Rect, c: Color) {
    let x0 = r.x();
    let y0 = r.y();
    let x1 = x0 + r.width() as i32;
    let y1 = y0 + r.height() as i32;
    for y in y0..y1 {
        for x in x0..x1 {
            px(img, x, y, c);
        }
    }
}

fn stroke_rect(img: &mut RgbaImage, r: Rect, c: Color, w: i32) {
    let x0 = r.x();
    let y0 = r.y();
    let x1 = x0 + r.width() as i32 - 1;
    let y1 = y0 + r.height() as i32 - 1;
    stroke_line(img, (x0, y0), (x1, y0), c, w);
    stroke_line(img, (x1, y0), (x1, y1), c, w);
    stroke_line(img, (x1, y1), (x0, y1), c, w);
    stroke_line(img, (x0, y1), (x0, y0), c, w);
}

fn stroke_ellipse(img: &mut RgbaImage, r: Rect, c: Color, _w: i32, fill: Option<Color>) {
    let cx = r.x() as f32 + r.width() as f32 / 2.0;
    let cy = r.y() as f32 + r.height() as f32 / 2.0;
    let rx = r.width() as f32 / 2.0;
    let ry = r.height() as f32 / 2.0;
    if rx == 0.0 || ry == 0.0 {
        return;
    }
    let circumference = (rx + ry) * f32::consts::TAU;
    let steps = circumference.ceil().max(64.0) as usize;
    let mut prev = ((cx + rx).round() as i32, (cy).round() as i32);
    for i in 1..=steps {
        let t = (i as f32 / steps as f32) * std::f32::consts::TAU;
        let x = (cx + rx * t.cos()).round() as i32;
        let y = (cy + ry * t.sin()).round() as i32;
        stroke_line(img, prev, (x, y), c, 1);
        prev = (x, y);
    }
    if let Some(fill) = fill {
        for y in r.y()..(r.y() + r.height() as i32) {
            for x in r.x()..(r.x() + r.width() as i32) {
                let nx = (x as f32 - cx) / rx;
                let ny = (y as f32 - cy) / ry;
                if nx * nx + ny * ny <= 1.0 {
                    px(img, x, y, fill);
                }
            }
        }
    }
}

fn draw_circle_outline(img: &mut RgbaImage, c: (i32, i32), r: i32, color: Color, w: i32) {
    let steps = (r as f32 * f32::consts::TAU).max(32.0) as usize;
    let mut prev = (c.0 + r, c.1);
    for i in 1..=steps {
        let t = (i as f32 / steps as f32) * std::f32::consts::TAU;
        let x = (c.0 as f32 + r as f32 * t.cos()).round() as i32;
        let y = (c.1 as f32 + r as f32 * t.sin()).round() as i32;
        stroke_line(img, prev, (x, y), color, w);
        prev = (x, y);
    }
}

fn draw_arrowhead(img: &mut RgbaImage, from: (i32, i32), to: (i32, i32), color: Color, w: i32) {
    let dx = (to.0 - from.0) as f32;
    let dy = (to.1 - from.1) as f32;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / len;
    let uy = dy / len;
    let head = (w as f32 * 3.0).max(10.0);
    let spread = 0.5;
    let p1 = (
        to.0 - (ux * head + uy * head * spread) as i32,
        to.1 - (uy * head - ux * head * spread) as i32,
    );
    let p2 = (
        to.0 - (ux * head - uy * head * spread) as i32,
        to.1 - (uy * head + ux * head * spread) as i32,
    );
    stroke_line(img, to, p1, color, w);
    stroke_line(img, to, p2, color, w);
}

fn apply_blur(img: &mut RgbaImage, rect: Rect, radius: f32, origin: (i32, i32)) {
    let r = local_rect(rect, origin);
    let (iw, ih) = img.dimensions();
    // Intersect with the image so a blur outside this monitor is a no-op
    // rather than leaking onto the wrong pixels.
    let x0 = r.x().max(0);
    let y0 = r.y().max(0);
    let x1 = (r.x() + r.width() as i32).min(iw as i32);
    let y1 = (r.y() + r.height() as i32).min(ih as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let x = x0 as u32;
    let y = y0 as u32;
    let w = (x1 - x0) as u32;
    let h = (y1 - y0) as u32;
    let cropped = imageops::crop_imm(img, x, y, w, h).to_image();
    let blurred = sss_core::blur::gaussian_blur(cropped, radius.max(1.0));
    imageops::replace(img, &blurred, x as i64, y as i64);
}

fn draw_number_centered(img: &mut RgbaImage, c: (i32, i32), n: u32, color: Color, radius: f32) {
    let s = n.to_string();
    let px = (radius * 1.1).max(8.0);
    let text_w = crate::font::measure(&s, px);
    let ascent = crate::font::ascent(px);
    let x0 = c.0 - (text_w / 2.0).round() as i32;
    let y0 = c.1 - (ascent / 2.0).round() as i32;
    crate::font::draw_text_rgba(img, x0, y0, &s, color.0, px);
}

fn draw_text(img: &mut RgbaImage, origin: (i32, i32), text: &str, style: &TextStyle) {
    crate::font::draw_text_rgba(img, origin.0, origin.1, text, style.color.0, style.size);
}
