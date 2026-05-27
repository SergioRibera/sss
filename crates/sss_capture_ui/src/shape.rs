//! Persistent shape model for the annotation canvas.

use crate::color::Color;
use crate::geometry::FPoint;
use crate::tool::{BrushSettings, StepSettings};
use sss_capture::Rect;

/// Strongly typed shape identifier; monotonic per canvas and never reused.
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
    /// Rotation in radians, around the bounding-box center.
    pub rotation: f32,
}

impl Shape {
    /// Axis-aligned bounding rectangle in canvas pixels.
    pub fn bounds(&self) -> Rect {
        self.kind.bounds()
    }

    pub fn contains(&self, p: FPoint) -> bool {
        crate::hit::shape_hit(self, p)
    }
}

#[derive(Clone, Debug)]
pub enum ShapeKind {
    FreehandStroke {
        points: Vec<FPoint>,
    },
    Line {
        from: FPoint,
        to: FPoint,
    },
    Arrow {
        from: FPoint,
        to: FPoint,
    },
    Rectangle {
        rect: Rect,
    },
    Ellipse {
        rect: Rect,
    },
    /// Rectangle whose interior is blurred during composition.
    BlurRect {
        rect: Rect,
        radius: f32,
    },
    Step {
        center: FPoint,
        number: u32,
        radius: f32,
    },
    Text {
        origin: FPoint,
        content: String,
        style: TextStyle,
    },
    /// Polygon; when `closed`, the interior is filled with `Style::fill`.
    Polygon {
        points: Vec<FPoint>,
        closed: bool,
    },
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

/// Visual style for non-text shapes.
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

/// Smooth a freehand polyline: drop clustered samples and densify with a
/// uniform Catmull–Rom spline. Output is suitable for direct rendering with
/// any polyline rasteriser — kinks vanish and slow-hand jitter is averaged
/// out without introducing visible drift away from input vertices.
pub fn smoothed_freehand(points: &[FPoint], width: f32) -> Vec<FPoint> {
    if points.len() < 2 {
        return points.to_vec();
    }
    let min_gap = (width * 0.4).max(0.75);
    let mut filtered: Vec<FPoint> = Vec::with_capacity(points.len());
    for &p in points {
        let push = match filtered.last() {
            Some(last) => last.distance(p) >= min_gap,
            None => true,
        };
        if push {
            filtered.push(p);
        }
    }
    // Always keep last raw sample so the rendered stroke reaches the cursor.
    if let Some(&last) = points.last() {
        if filtered.last().map(|p| *p != last).unwrap_or(true) {
            filtered.push(last);
        }
    }
    if filtered.len() < 3 {
        return filtered;
    }
    let n = filtered.len();
    let mut out: Vec<FPoint> = Vec::with_capacity(n * 8);
    out.push(filtered[0]);
    for i in 0..n - 1 {
        let p0 = if i == 0 { filtered[0] } else { filtered[i - 1] };
        let p1 = filtered[i];
        let p2 = filtered[i + 1];
        let p3 = if i + 2 >= n {
            filtered[n - 1]
        } else {
            filtered[i + 2]
        };
        // Adapt sample count to segment length so short hops don't waste verts.
        let dist = p1.distance(p2);
        let segments = (dist.ceil() as usize / 2).clamp(2, 10);
        for s in 1..=segments {
            let t = s as f32 / segments as f32;
            out.push(catmull_rom(p0, p1, p2, p3, t));
        }
    }
    out
}

fn catmull_rom(p0: FPoint, p1: FPoint, p2: FPoint, p3: FPoint, t: f32) -> FPoint {
    let t2 = t * t;
    let t3 = t2 * t;
    let x = 0.5
        * ((2.0 * p1.x)
            + (-p0.x + p2.x) * t
            + (2.0 * p0.x - 5.0 * p1.x + 4.0 * p2.x - p3.x) * t2
            + (-p0.x + 3.0 * p1.x - 3.0 * p2.x + p3.x) * t3);
    let y = 0.5
        * ((2.0 * p1.y)
            + (-p0.y + p2.y) * t
            + (2.0 * p0.y - 5.0 * p1.y + 4.0 * p2.y - p3.y) * t2
            + (-p0.y + 3.0 * p1.y - 3.0 * p2.y + p3.y) * t3);
    FPoint::new(x, y)
}
