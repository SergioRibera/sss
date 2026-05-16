//! Annotation tools the overlay exposes through the toolbar.
//!
//! Every variant is plain data; the rendering logic that interprets them
//! lives under [`crate::render`]. Keeping it that way means the editor's
//! state can be serialised, replayed, fuzzed, or driven by a script.

use crate::color::Color;

/// What the user is currently doing with the pointer.
///
/// The order in this enum matches the visual order in the default toolbar.
#[derive(Clone, Debug, PartialEq)]
pub enum Tool {
    /// Selection / re-edit tool. Clicking on an existing shape grabs it for
    /// move / resize / restyle. Dragging in empty space adjusts the active
    /// region rectangle when the user is in [`crate::SelectorMode::Area`].
    Pointer,

    /// Freehand brush. Each drag emits a [`ShapeKind::FreehandStroke`].
    Brush(BrushSettings),

    /// Straight line.
    Line(BrushSettings),

    /// Arrow with a triangular head at the *to* end.
    Arrow(BrushSettings),

    /// Hollow rectangle outline.
    Rectangle(BrushSettings),

    /// Hollow ellipse / circle outline.
    Ellipse(BrushSettings),

    /// Rectangle whose interior is blurred (Gaussian) when the final image
    /// is composited.
    BlurRect { radius: f32 },

    /// Removes any shape that intersects the eraser radius.
    Eraser { radius: f32 },

    /// Numbered circle. Each click places the next number in sequence.
    Step(StepSettings),

    /// Text label.
    Text(crate::shape::TextStyle),

    /// Multi-vertex polygon. Click adds a vertex; right-click (or pressing
    /// Enter) closes the polygon and commits it as a shape. Closed
    /// polygons honour the FILL toggle the same way Rectangle / Ellipse
    /// do.
    Polygon(BrushSettings),
}

impl Tool {
    pub fn name(&self) -> &'static str {
        match self {
            Tool::Pointer => "Pointer",
            Tool::Brush(_) => "Brush",
            Tool::Line(_) => "Line",
            Tool::Arrow(_) => "Arrow",
            Tool::Rectangle(_) => "Rectangle",
            Tool::Ellipse(_) => "Ellipse",
            Tool::BlurRect { .. } => "Blur",
            Tool::Eraser { .. } => "Eraser",
            Tool::Step(_) => "Step",
            Tool::Text(_) => "Text",
            Tool::Polygon(_) => "Polygon",
        }
    }

    pub fn icon(&self) -> &'static str {
        // Single-glyph icons used by the egui toolbar. ASCII-only so the
        // default font renders them without needing a font pack.
        match self {
            Tool::Pointer => "↖",
            Tool::Brush(_) => "✎",
            Tool::Line(_) => "／",
            Tool::Arrow(_) => "➜",
            Tool::Rectangle(_) => "▭",
            Tool::Ellipse(_) => "◯",
            Tool::BlurRect { .. } => "▓",
            Tool::Eraser { .. } => "⌫",
            Tool::Step(_) => "①",
            Tool::Text(_) => "T",
            Tool::Polygon(_) => "⬠",
        }
    }
}

impl Default for Tool {
    fn default() -> Self {
        Tool::Pointer
    }
}

/// Shared settings used by every stroke / shape that draws a colored line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrushSettings {
    pub color: Color,
    /// Stroke width in logical pixels.
    pub width: f32,
    /// Whether to fill the interior (for closed shapes only).
    pub fill: Option<Color>,
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self {
            color: Color::RED,
            width: 3.0,
            fill: None,
        }
    }
}

impl BrushSettings {
    pub const fn solid(color: Color, width: f32) -> Self {
        Self {
            color,
            width,
            fill: None,
        }
    }
}

/// Settings for the numbered-step tool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StepSettings {
    pub fill: Color,
    pub text: Color,
    pub radius: f32,
    pub next_number: u32,
}

impl Default for StepSettings {
    fn default() -> Self {
        Self {
            fill: Color::RED,
            text: Color::WHITE,
            radius: 14.0,
            next_number: 1,
        }
    }
}

/// Subset of [`Tool`] variants that should appear in the toolbar.
///
/// Hosts can hide tools or pre-fill them with custom defaults. The order of
/// the slice is the visual order in the toolbar.
#[derive(Clone, Debug)]
pub struct ToolPalette {
    pub tools: Vec<Tool>,
    pub color_palette: Vec<Color>,
    pub initial: Tool,
}

impl Default for ToolPalette {
    fn default() -> Self {
        Self {
            tools: vec![
                Tool::Pointer,
                Tool::Brush(BrushSettings::default()),
                Tool::Line(BrushSettings::default()),
                Tool::Arrow(BrushSettings::default()),
                Tool::Rectangle(BrushSettings::default()),
                Tool::Ellipse(BrushSettings::default()),
                Tool::Polygon(BrushSettings::default()),
                Tool::BlurRect { radius: 12.0 },
                Tool::Eraser { radius: 18.0 },
                Tool::Step(StepSettings::default()),
            ],
            color_palette: Color::palette().to_vec(),
            initial: Tool::Pointer,
        }
    }
}

impl ToolPalette {
    /// A slimmed-down palette without annotation tools — only Pointer.
    /// Useful when wiring up the `sss-select` slurp-equivalent binary.
    pub fn minimal() -> Self {
        Self {
            tools: vec![Tool::Pointer],
            color_palette: Vec::new(),
            initial: Tool::Pointer,
        }
    }
}
