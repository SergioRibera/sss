//! Annotation tools exposed by the overlay toolbar.

use crate::color::Color;

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Tool {
    #[default]
    Pointer,
    Brush(BrushSettings),
    Line(BrushSettings),
    Arrow(BrushSettings),
    Rectangle(BrushSettings),
    Ellipse(BrushSettings),
    /// Rectangle whose interior is blurred during composition.
    BlurRect {
        radius: f32,
    },
    /// Removes any shape that intersects the eraser radius.
    Eraser {
        radius: f32,
    },
    Step(StepSettings),
    Text(crate::shape::TextStyle),
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

/// Settings shared by stroke-drawing tools.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrushSettings {
    pub color: Color,
    pub width: f32,
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

/// Toolbar configuration; `tools` is the visual order in the toolbar.
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
    /// Pointer-only palette without annotation tools.
    pub fn minimal() -> Self {
        Self {
            tools: vec![Tool::Pointer],
            color_palette: Vec::new(),
            initial: Tool::Pointer,
        }
    }
}
