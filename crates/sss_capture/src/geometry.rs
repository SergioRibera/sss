//! Geometry primitives in the logical desktop coordinate space.

use std::fmt;

/// A 2D point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const ORIGIN: Self = Self { x: 0, y: 0 };

    #[inline]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

impl From<(i32, i32)> for Point {
    fn from((x, y): (i32, i32)) -> Self {
        Self { x, y }
    }
}

/// A 2D size in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    #[inline]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    #[inline]
    pub const fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

impl From<(u32, u32)> for Size {
    fn from((w, h): (u32, u32)) -> Self {
        Self::new(w, h)
    }
}

/// An axis-aligned rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    #[inline]
    pub const fn new(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    #[inline]
    pub const fn from_xywh(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self::new(Point::new(x, y), Size::new(width, height))
    }

    /// A 1×1 rectangle at the given point.
    #[inline]
    pub const fn point(x: i32, y: i32) -> Self {
        Self::from_xywh(x, y, 1, 1)
    }

    #[inline]
    pub const fn x(&self) -> i32 {
        self.origin.x
    }
    #[inline]
    pub const fn y(&self) -> i32 {
        self.origin.y
    }
    #[inline]
    pub const fn width(&self) -> u32 {
        self.size.width
    }
    #[inline]
    pub const fn height(&self) -> u32 {
        self.size.height
    }

    #[inline]
    pub fn right(&self) -> i32 {
        self.origin.x.saturating_add(self.size.width as i32)
    }
    #[inline]
    pub fn bottom(&self) -> i32 {
        self.origin.y.saturating_add(self.size.height as i32)
    }

    #[inline]
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.origin.x && p.x < self.right() && p.y >= self.origin.y && p.y < self.bottom()
    }

    /// Geometric intersection; touching edges count as disjoint.
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x0 = self.origin.x.max(other.origin.x);
        let y0 = self.origin.y.max(other.origin.y);
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        if x1 > x0 && y1 > y0 {
            Some(Rect::from_xywh(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32))
        } else {
            None
        }
    }

    /// Bounding rectangle (union) of a non-empty slice of rectangles.
    pub fn bounding(rects: &[Rect]) -> Option<Rect> {
        let first = rects.first()?;
        let mut x0 = first.origin.x;
        let mut y0 = first.origin.y;
        let mut x1 = first.right();
        let mut y1 = first.bottom();
        for r in &rects[1..] {
            x0 = x0.min(r.origin.x);
            y0 = y0.min(r.origin.y);
            x1 = x1.max(r.right());
            y1 = y1.max(r.bottom());
        }
        Some(Rect::from_xywh(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32))
    }
}

impl fmt::Display for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{},{} {}x{}",
            self.origin.x, self.origin.y, self.size.width, self.size.height
        )
    }
}

impl From<(i32, i32, u32, u32)> for Rect {
    fn from((x, y, w, h): (i32, i32, u32, u32)) -> Self {
        Self::from_xywh(x, y, w, h)
    }
}

pub type Area = Rect;

/// Output transform mirroring Wayland's `wl_output.transform`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Rotation {
    #[default]
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

impl Rotation {
    pub fn from_degrees(d: f32) -> Self {
        let d = d.rem_euclid(360.0);
        if (d - 90.0).abs() < f32::EPSILON {
            Self::Rotate90
        } else if (d - 180.0).abs() < f32::EPSILON {
            Self::Rotate180
        } else if (d - 270.0).abs() < f32::EPSILON {
            Self::Rotate270
        } else {
            Self::Normal
        }
    }

    pub const fn degrees(self) -> f32 {
        match self {
            Self::Normal | Self::Flipped => 0.0,
            Self::Rotate90 | Self::Flipped90 => 90.0,
            Self::Rotate180 | Self::Flipped180 => 180.0,
            Self::Rotate270 | Self::Flipped270 => 270.0,
        }
    }

    pub const fn is_flipped(self) -> bool {
        matches!(
            self,
            Self::Flipped | Self::Flipped90 | Self::Flipped180 | Self::Flipped270
        )
    }
}

impl From<f32> for Rotation {
    fn from(value: f32) -> Self {
        Self::from_degrees(value)
    }
}

impl From<i32> for Rotation {
    fn from(value: i32) -> Self {
        Self::from_degrees(value as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersection_disjoint() {
        let a = Rect::from_xywh(0, 0, 100, 100);
        let b = Rect::from_xywh(200, 0, 100, 100);
        assert_eq!(a.intersection(&b), None);
    }

    #[test]
    fn intersection_overlap() {
        let a = Rect::from_xywh(0, 0, 100, 100);
        let b = Rect::from_xywh(50, 50, 100, 100);
        assert_eq!(a.intersection(&b), Some(Rect::from_xywh(50, 50, 50, 50)));
    }

    #[test]
    fn intersection_touch_is_disjoint() {
        let a = Rect::from_xywh(0, 0, 100, 100);
        let b = Rect::from_xywh(100, 0, 100, 100);
        assert_eq!(a.intersection(&b), None);
    }

    #[test]
    fn bounding_two_monitors() {
        let m1 = Rect::from_xywh(0, 0, 1920, 1080);
        let m2 = Rect::from_xywh(1920, 0, 2560, 1440);
        assert_eq!(
            Rect::bounding(&[m1, m2]),
            Some(Rect::from_xywh(0, 0, 4480, 1440))
        );
    }

    #[test]
    fn contains_open_right_bottom() {
        let r = Rect::from_xywh(0, 0, 10, 10);
        assert!(r.contains(Point::new(0, 0)));
        assert!(r.contains(Point::new(9, 9)));
        assert!(!r.contains(Point::new(10, 5)));
        assert!(!r.contains(Point::new(5, 10)));
    }

    #[test]
    fn rotation_round_trip() {
        assert_eq!(Rotation::from_degrees(0.0), Rotation::Normal);
        assert_eq!(Rotation::from_degrees(90.0), Rotation::Rotate90);
        assert_eq!(Rotation::from_degrees(180.0), Rotation::Rotate180);
        assert_eq!(Rotation::from_degrees(270.0), Rotation::Rotate270);
        assert_eq!(Rotation::from_degrees(-90.0), Rotation::Rotate270);
    }
}
