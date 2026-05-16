//! Floating-point geometry helpers used during interactive editing.
//!
//! `sss_capture::Rect` / `Point` are integer-only — fine for the final
//! committed selection but inconvenient during a drag (sub-pixel cursor
//! deltas, smooth brush strokes). `FPoint` and `FRect` cover the editing
//! path; they convert to/from the integer flavours at the boundaries.

use sss_capture::{Point, Rect};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FPoint {
    pub x: f32,
    pub y: f32,
}

impl FPoint {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn to_int(self) -> Point {
        Point::new(self.x.round() as i32, self.y.round() as i32)
    }
    pub fn from_int(p: Point) -> Self {
        Self {
            x: p.x as f32,
            y: p.y as f32,
        }
    }
    pub fn distance(self, other: FPoint) -> f32 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

impl From<(f32, f32)> for FPoint {
    fn from((x, y): (f32, f32)) -> Self {
        Self { x, y }
    }
}

impl From<Point> for FPoint {
    fn from(p: Point) -> Self {
        Self::from_int(p)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl FRect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    /// Build from two opposite corners (drag start + drag current). The
    /// corners may be in any order.
    pub fn from_corners(a: FPoint, b: FPoint) -> Self {
        let x0 = a.x.min(b.x);
        let y0 = a.y.min(b.y);
        let x1 = a.x.max(b.x);
        let y1 = a.y.max(b.y);
        Self {
            x: x0,
            y: y0,
            w: (x1 - x0).max(0.0),
            h: (y1 - y0).max(0.0),
        }
    }
    pub fn to_int(self) -> Rect {
        Rect::from_xywh(
            self.x.round() as i32,
            self.y.round() as i32,
            self.w.round().max(0.0) as u32,
            self.h.round().max(0.0) as u32,
        )
    }
}

impl From<Rect> for FRect {
    fn from(r: Rect) -> Self {
        Self::new(
            r.x() as f32,
            r.y() as f32,
            r.width() as f32,
            r.height() as f32,
        )
    }
}
