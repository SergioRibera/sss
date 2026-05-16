//! CPU rasteriser used to bake the final image.
//!
//! Every shape from [`Canvas::shapes`] gets drawn over the source RGBA
//! buffer. After all stroked shapes are drawn, we apply the BlurRect regions
//! by Gaussian-blurring the captured pixels under each rectangle.
//!
//! Drawing is done with simple software primitives (Bresenham lines,
//! filled / outlined rects, midpoint ellipses, dot fonts for steps/text).
//! It's not a vector graphics library — accuracy is "good enough for a
//! screenshot annotation overlay" — but the output is fully deterministic
//! and matches what the interactive preview shows.

use image::imageops;
use image::{Rgba, RgbaImage};

use crate::canvas::Canvas;
use crate::color::Color;
use crate::geometry::FPoint;
use crate::shape::{Shape, ShapeKind, TextStyle};
use sss_capture::Rect;

/// Render every shape in `canvas` onto `image`. `origin` is the canvas
/// coordinate the image's top-left corresponds to.
///
/// Shapes are processed in z-order (the order they were committed), so a
/// stroke drawn *after* a blur rectangle paints on top of the blur, and a
/// blur drawn *after* a stroke obscures it. This matches the user's
/// expectation of "what I drew last wins".
pub fn flatten(image: &mut RgbaImage, canvas: &Canvas, origin: (i32, i32)) {
    for shape in canvas.shapes() {
        paint_one(image, shape, origin);
    }
}

/// Same as [`flatten`] but also paints the in-flight drag preview and any
/// pending text the user is typing. Used by the *interactive* renderers
/// so the drawn shape appears live as the user drags / types, in addition
/// to the committed shapes already on the canvas.
///
/// The preview is appended *after* every committed shape so it always sits
/// on top while it's still being drawn.
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
    // In-flight polygon: draw the committed vertices as an open polyline
    // so the user can see what they're building. The renderer keeps the
    // polygon "open" (the closing segment from last → first is only
    // drawn when the user commits).
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
        // Mark each committed vertex with a small filled square so the
        // user can see them clearly while the polygon is being built.
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
        BlurRect { .. } => {
            // Handled in second pass.
        }
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
                draw_number_centered(img, c, *number, Color::WHITE);
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
            // Fill first so the outline sits on top (matches every
            // other vector tool in the editor).
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

/// Scanline-fill a (possibly non-convex) polygon. Slow but correct for any
/// vertex order; we only use it for tens of vertices at most.
fn fill_polygon(img: &mut RgbaImage, points: &[FPoint], origin: (i32, i32), color: Color) {
    if points.len() < 3 {
        return;
    }
    // Convert all vertices to image-local coords up-front.
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

// ---------------------------------------------------------------------------
// Primitive helpers
// ---------------------------------------------------------------------------

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

/// Source-over alpha blending.
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

/// Anti-aliased thick line.
///
/// Distance-based coverage: each pixel within the line's half-width gets
/// full coverage, pixels in the 1-px feather band get a linear ramp, and
/// everything outside is untouched. Works equally well for thin lines
/// (the brush at 1 px is effectively Wu-style AA) and thick brush strokes,
/// and round-caps + joins are implicit (every point on the polyline shares
/// pixels with its neighbours).
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
    // Bresenham with thickness; for thick strokes we expand into a disk at
    // each visited point. Slow but trivial — fine for an annotation overlay.
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
    // Parametric ellipse — good enough for screenshot annotations and
    // doesn't depend on the rasteriser quality of any external library.
    let cx = r.x() as f32 + r.width() as f32 / 2.0;
    let cy = r.y() as f32 + r.height() as f32 / 2.0;
    let rx = r.width() as f32 / 2.0;
    let ry = r.height() as f32 / 2.0;
    if rx == 0.0 || ry == 0.0 {
        return;
    }
    let circumference = (rx + ry) * 6.28;
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
        // Naive fill: iterate the bounding box, test ellipse equation.
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
    let steps = (r as f32 * 6.28).max(32.0) as usize;
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

// ---------------------------------------------------------------------------
// Blur
// ---------------------------------------------------------------------------

fn apply_blur(img: &mut RgbaImage, rect: Rect, radius: f32, origin: (i32, i32)) {
    let r = local_rect(rect, origin);
    let (iw, ih) = img.dimensions();
    // Proper rectangle-with-image intersection. The old version clamped
    // `x`/`y` to 0 *without* tracking how much of the rectangle was
    // chopped off the left / top — so a blur whose bounds sat entirely
    // *outside* this image (e.g. on a different monitor) ended up
    // blurring the wrong pixels because `width` was still the original
    // and `x` had snapped to 0. The user saw blur leak onto the next
    // monitor, and the leak followed the rect as they dragged it.
    let x0 = r.x().max(0);
    let y0 = r.y().max(0);
    let x1 = (r.x() + r.width() as i32).min(iw as i32);
    let y1 = (r.y() + r.height() as i32).min(ih as i32);
    if x1 <= x0 || y1 <= y0 {
        // No intersection with this image — nothing to blur on this monitor.
        return;
    }
    let x = x0 as u32;
    let y = y0 as u32;
    let w = (x1 - x0) as u32;
    let h = (y1 - y0) as u32;
    let cropped = imageops::crop_imm(img, x, y, w, h).to_image();
    let blurred = imageops::blur(&cropped, radius.max(1.0));
    imageops::replace(img, &blurred, x as i64, y as i64);
}

// ---------------------------------------------------------------------------
// Tiny "font" for step numbers & text
// ---------------------------------------------------------------------------
//
// We embed a 5×7 bitmap for digits and a basic ASCII subset. This is plenty
// for numeric step markers and short text labels; users who need rich text
// can flatten the canvas client-side and use any font they like.

const GLYPH_W: usize = 5;
const GLYPH_H: usize = 7;

fn draw_number_centered(img: &mut RgbaImage, c: (i32, i32), n: u32, color: Color) {
    let s = n.to_string();
    let total_w = s.len() as i32 * (GLYPH_W as i32 + 1) - 1;
    let x0 = c.0 - total_w / 2;
    let y0 = c.1 - GLYPH_H as i32 / 2;
    for (i, ch) in s.chars().enumerate() {
        draw_glyph(img, x0 + i as i32 * (GLYPH_W as i32 + 1), y0, ch, color, 1);
    }
}

fn draw_text(img: &mut RgbaImage, origin: (i32, i32), text: &str, style: &TextStyle) {
    let scale = (style.size / GLYPH_H as f32).max(1.0).round() as i32;
    let mut x = origin.0;
    for ch in text.chars() {
        draw_glyph(img, x, origin.1, ch, style.color, scale);
        x += (GLYPH_W as i32 + 1) * scale;
    }
}

fn draw_glyph(img: &mut RgbaImage, x: i32, y: i32, ch: char, color: Color, scale: i32) {
    let bits = glyph_bits(ch);
    for row in 0..GLYPH_H {
        for col in 0..GLYPH_W {
            if bits[row] & (1 << (GLYPH_W - 1 - col)) != 0 {
                for dy in 0..scale {
                    for dx in 0..scale {
                        px(
                            img,
                            x + col as i32 * scale + dx,
                            y + row as i32 * scale + dy,
                            color,
                        );
                    }
                }
            }
        }
    }
}

fn glyph_bits(c: char) -> [u8; 7] {
    match c {
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        '6' => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
        ],
        ' ' => [0; 7],
        'A'..='Z' => letter_bits(c),
        'a'..='z' => letter_bits(c.to_ascii_uppercase()),
        '.' => [0, 0, 0, 0, 0, 0b00110, 0b00110],
        ',' => [0, 0, 0, 0, 0, 0b00110, 0b00100],
        '!' => [
            0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100,
        ],
        '?' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b00000, 0b00100,
        ],
        ':' => [0, 0b00110, 0b00110, 0, 0b00110, 0b00110, 0],
        '-' => [0, 0, 0, 0b11111, 0, 0, 0],
        '/' => [0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0, 0],
        _ => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b00000, 0b00100,
        ],
    }
}

fn letter_bits(c: char) -> [u8; 7] {
    match c {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01110, 0b10001, 0b10000, 0b01110, 0b00001, 0b10001, 0b01110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        _ => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b00000, 0b00100,
        ],
    }
}
