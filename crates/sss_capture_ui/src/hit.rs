//! Pointer hit-testing against shapes.
//!
//! Implements the click → which shape did I hit? lookup the Pointer tool
//! uses for selection / move / restyle.

use crate::geometry::FPoint;
use crate::shape::{Shape, ShapeKind};

/// Tolerance (in pixels) added to thin shapes so users can grab them with
/// the cursor without millimetre precision.
const STROKE_PAD: f32 = 5.0;

pub fn shape_hit(shape: &Shape, p: FPoint) -> bool {
    use ShapeKind::*;
    let pad = shape.style.stroke_width.max(STROKE_PAD);
    match &shape.kind {
        FreehandStroke { points } => points
            .windows(2)
            .any(|s| dist_point_to_segment(p, s[0], s[1]) <= pad),
        Line { from, to } | Arrow { from, to } => dist_point_to_segment(p, *from, *to) <= pad,
        Rectangle { rect } | BlurRect { rect, .. } => {
            // Outline (or interior if filled) hit area.
            if shape.style.fill.is_some() {
                rect_contains(rect, p)
            } else {
                rect_outline_hit(rect, p, pad)
            }
        }
        Ellipse { rect } => {
            if shape.style.fill.is_some() {
                ellipse_contains(rect, p)
            } else {
                ellipse_outline_hit(rect, p, pad)
            }
        }
        Step { center, radius, .. } => p.distance(*center) <= *radius + pad,
        Text {
            origin,
            content,
            style,
        } => {
            let w = content.chars().count() as f32 * style.size * 0.55;
            let h = style.size;
            p.x >= origin.x && p.x <= origin.x + w && p.y >= origin.y && p.y <= origin.y + h
        }
        Polygon { points, closed } => {
            if shape.style.fill.is_some() && *closed && point_in_polygon(points, p) {
                return true;
            }
            // Otherwise treat as a polyline: hit if the cursor is near any edge.
            let mut last = None;
            for v in points {
                if let Some(prev) = last {
                    if dist_point_to_segment(p, prev, *v) <= pad {
                        return true;
                    }
                }
                last = Some(*v);
            }
            if *closed && points.len() >= 3 {
                if let (Some(first), Some(last)) = (points.first(), points.last()) {
                    if dist_point_to_segment(p, *last, *first) <= pad {
                        return true;
                    }
                }
            }
            false
        }
    }
}

fn point_in_polygon(points: &[FPoint], p: FPoint) -> bool {
    // Ray-cast even-odd rule.
    let mut inside = false;
    let n = points.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let xi = points[i].x;
        let yi = points[i].y;
        let xj = points[j].x;
        let yj = points[j].y;
        let intersect = (yi > p.y) != (yj > p.y)
            && p.x < (xj - xi) * (p.y - yi) / (yj - yi + f32::EPSILON) + xi;
        if intersect {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn rect_contains(r: &sss_capture::Rect, p: FPoint) -> bool {
    let x0 = r.x() as f32;
    let y0 = r.y() as f32;
    let x1 = x0 + r.width() as f32;
    let y1 = y0 + r.height() as f32;
    p.x >= x0 && p.x <= x1 && p.y >= y0 && p.y <= y1
}

fn rect_outline_hit(r: &sss_capture::Rect, p: FPoint, pad: f32) -> bool {
    // Inside the rect but not deep inside it.
    if !rect_contains(r, p) {
        // Outside but close to an edge?
        let x0 = r.x() as f32;
        let y0 = r.y() as f32;
        let x1 = x0 + r.width() as f32;
        let y1 = y0 + r.height() as f32;
        return p.x >= x0 - pad && p.x <= x1 + pad && p.y >= y0 - pad && p.y <= y1 + pad;
    }
    let x0 = r.x() as f32;
    let y0 = r.y() as f32;
    let x1 = x0 + r.width() as f32;
    let y1 = y0 + r.height() as f32;
    (p.x - x0).abs() <= pad
        || (p.x - x1).abs() <= pad
        || (p.y - y0).abs() <= pad
        || (p.y - y1).abs() <= pad
}

fn ellipse_contains(r: &sss_capture::Rect, p: FPoint) -> bool {
    let cx = r.x() as f32 + r.width() as f32 / 2.0;
    let cy = r.y() as f32 + r.height() as f32 / 2.0;
    let rx = r.width() as f32 / 2.0;
    let ry = r.height() as f32 / 2.0;
    if rx == 0.0 || ry == 0.0 {
        return false;
    }
    let dx = (p.x - cx) / rx;
    let dy = (p.y - cy) / ry;
    dx * dx + dy * dy <= 1.0
}

fn ellipse_outline_hit(r: &sss_capture::Rect, p: FPoint, pad: f32) -> bool {
    // Difference of two ellipses: inside the outer, outside the inner.
    let cx = r.x() as f32 + r.width() as f32 / 2.0;
    let cy = r.y() as f32 + r.height() as f32 / 2.0;
    let rx = r.width() as f32 / 2.0;
    let ry = r.height() as f32 / 2.0;
    if rx <= pad || ry <= pad {
        return false;
    }
    let outer = {
        let dx = (p.x - cx) / (rx + pad);
        let dy = (p.y - cy) / (ry + pad);
        dx * dx + dy * dy <= 1.0
    };
    let inner = {
        let dx = (p.x - cx) / (rx - pad);
        let dy = (p.y - cy) / (ry - pad);
        dx * dx + dy * dy <= 1.0
    };
    outer && !inner
}

fn dist_point_to_segment(p: FPoint, a: FPoint, b: FPoint) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    let proj = FPoint::new(a.x + t * dx, a.y + t * dy);
    p.distance(proj)
}
