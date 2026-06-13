//! Shared OCR detection types used by `sss_ocr` (producer) and
//! `sss_capture_ui` (consumer / renderer).
//!
//! Lives in `sss_core` so neither downstream crate has to depend on the
//! other: the producer maps its engine output into [`TextBox`], the UI
//! maps user interaction back from polygon hit-tests.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A 2D point in the captured image's pixel coordinate space.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TextPoint {
    pub x: f32,
    pub y: f32,
}

/// A single OCR detection: polygon + recognised text + confidence.
///
/// `polygon` may have 4+ points. PaddleOCR usually returns axis-aligned
/// quads but rotated text produces arbitrary convex polygons. For simple
/// rectangle hit-testing use [`Self::aabb`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TextBox {
    pub polygon: Vec<TextPoint>,
    pub text: String,
    pub confidence: f32,
    /// Region label assigned by the recogniser. `"formula"` when the
    /// formula model produced this box, `"text"` (or empty) otherwise.
    pub label: String,
}

impl TextBox {
    /// Axis-aligned bounding box `(x, y, w, h)` over [`Self::polygon`].
    pub fn aabb(&self) -> (f32, f32, f32, f32) {
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for p in &self.polygon {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
        (
            min_x,
            min_y,
            (max_x - min_x).max(0.0),
            (max_y - min_y).max(0.0),
        )
    }

    /// True when `(x, y)` falls inside the axis-aligned bounds.
    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        let (bx, by, w, h) = self.aabb();
        x >= bx && y >= by && x <= bx + w && y <= by + h
    }

    pub fn is_formula(&self) -> bool {
        self.label.eq_ignore_ascii_case("formula")
    }
}
