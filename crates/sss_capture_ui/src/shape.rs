//! Persistent shape model.
//!
//! Each completed action ("draw an arrow", "place a step", "type a label",
//! "blur this rectangle") becomes a [`Shape`] stored in the [`Canvas`].
//! Shapes are *editable* through the Pointer tool — they expose handles, can
//! be moved, resized, restyled, deleted or pulled up/down in z-order.
//!
//! Anything you can express here is also what gets baked into the final
//! composite image when the user confirms the selection.

use crate::color::Color;
use crate::geometry::FPoint;
use crate::tool::{BrushSettings, StepSettings};
use sss_capture::Rect;

mod inline {
    // Make sure `geometry::FPoint` is reachable from this file regardless of
    // the module layout (used as a re-export indirection).
}

/// Strongly typed shape identifier. Monotonic per canvas; never reused even
/// after a shape is deleted, so undo/redo can refer to gone shapes safely.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShapeId(pub(crate) u64);

impl ShapeId {
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// A single editable artefact on the canvas.
#[derive(Clone, Debug)]
pub struct Shape {
    pub id: ShapeId,
    pub kind: ShapeKind,
    pub style: Style,
    /// Optional rotation in radians, around the shape's bounding-box center.
    pub rotation: f32,
}

impl Shape {
    /// Axis-aligned bounding rectangle, in canvas (logical desktop) pixels.
    pub fn bounds(&self) -> Rect {
        self.kind.bounds()
    }

    /// True if `p` falls within the *hit area* of this shape — accounting for
    /// stroke width for outlines.
    pub fn contains(&self, p: FPoint) -> bool {
        crate::hit::shape_hit(self, p)
    }
}

#[derive(Clone, Debug)]
pub enum ShapeKind {
    /// Freehand brush stroke; the polyline of mouse samples.
    FreehandStroke { points: Vec<FPoint> },
    /// Straight line.
    Line { from: FPoint, to: FPoint },
    /// Arrow with a head at `to`.
    Arrow { from: FPoint, to: FPoint },
    /// Rectangle outline (and optional fill).
    Rectangle { rect: Rect },
    /// Ellipse outline (and optional fill).
    Ellipse { rect: Rect },
    /// Rectangle whose interior is blurred during composition.
    BlurRect { rect: Rect, radius: f32 },
    /// Numbered circle.
    Step {
        center: FPoint,
        number: u32,
        radius: f32,
    },
    /// Text label.
    Text {
        origin: FPoint,
        content: String,
        style: TextStyle,
    },
    /// Polygon defined by an ordered list of vertices. When `closed` is
    /// true the outline wraps from the last point back to the first and
    /// the interior is filled with `Style::fill`.
    Polygon { points: Vec<FPoint>, closed: bool },
}

impl ShapeKind {
    pub fn bounds(&self) -> Rect {
        use ShapeKind::*;
        match self {
            FreehandStroke { points } => bounding_of_points(points),
            Polygon { points, .. } => bounding_of_points(points),
            Line { from, to } | Arrow { from, to } => bounding_of_points(&[*from, *to]),
            Rectangle { rect } | Ellipse { rect } | BlurRect { rect, .. } => *rect,
            Step { center, radius, .. } => {
                let r = *radius;
                Rect::from_xywh(
                    (center.x - r) as i32,
                    (center.y - r) as i32,
                    (r * 2.0).ceil() as u32,
                    (r * 2.0).ceil() as u32,
                )
            }
            Text {
                origin,
                content,
                style,
            } => {
                let w = (content.chars().count() as f32 * style.size * 0.55).ceil() as u32;
                let h = style.size.ceil() as u32;
                Rect::from_xywh(origin.x as i32, origin.y as i32, w.max(1), h.max(1))
            }
        }
    }
}

fn bounding_of_points(pts: &[FPoint]) -> Rect {
    if pts.is_empty() {
        return Rect::default();
    }
    let mut x0 = pts[0].x;
    let mut y0 = pts[0].y;
    let mut x1 = pts[0].x;
    let mut y1 = pts[0].y;
    for p in &pts[1..] {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    Rect::from_xywh(
        x0.floor() as i32,
        y0.floor() as i32,
        (x1 - x0).ceil().max(1.0) as u32,
        (y1 - y0).ceil().max(1.0) as u32,
    )
}

/// Visual style shared by every non-text shape. Text shapes carry their own
/// [`TextStyle`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub stroke: Color,
    pub stroke_width: f32,
    pub fill: Option<Color>,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            stroke: Color::RED,
            stroke_width: 3.0,
            fill: None,
        }
    }
}

impl From<BrushSettings> for Style {
    fn from(b: BrushSettings) -> Self {
        Self {
            stroke: b.color,
            stroke_width: b.width,
            fill: b.fill,
        }
    }
}

impl From<StepSettings> for Style {
    fn from(s: StepSettings) -> Self {
        Self {
            stroke: s.fill,
            stroke_width: 0.0,
            fill: Some(s.fill),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub color: Color,
    pub size: f32,
    pub bold: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            color: Color::RED,
            size: 18.0,
            bold: false,
        }
    }
}
