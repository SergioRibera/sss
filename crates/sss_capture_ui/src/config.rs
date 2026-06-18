//! User-facing configuration for the interactive overlay.

use crate::color::Color;
use crate::tool::{BrushSettings, StepSettings, Tool, ToolPalette};

/// Serializable identifier for a `Tool` variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum ToolKind {
    Pointer,
    Brush,
    Line,
    Arrow,
    Rectangle,
    Ellipse,
    Polygon,
    BlurRect,
    Eraser,
    Step,
    Text,
}

impl ToolKind {
    pub fn default_list() -> Vec<ToolKind> {
        vec![
            ToolKind::Pointer,
            ToolKind::Brush,
            ToolKind::Line,
            ToolKind::Arrow,
            ToolKind::Rectangle,
            ToolKind::Ellipse,
            ToolKind::Polygon,
            ToolKind::BlurRect,
            ToolKind::Eraser,
            ToolKind::Step,
        ]
    }

    pub fn build(self, ui: &UiConfig) -> Tool {
        let brush = BrushSettings {
            color: ui.default_stroke_color,
            width: ui.default_stroke_width.max(0.5),
            fill: ui.default_fill,
        };
        match self {
            ToolKind::Pointer => Tool::Pointer,
            ToolKind::Brush => Tool::Brush(brush),
            ToolKind::Line => Tool::Line(brush),
            ToolKind::Arrow => Tool::Arrow(brush),
            ToolKind::Rectangle => Tool::Rectangle(brush),
            ToolKind::Ellipse => Tool::Ellipse(brush),
            ToolKind::Polygon => Tool::Polygon(brush),
            ToolKind::BlurRect => Tool::BlurRect {
                radius: ui.default_blur_radius,
            },
            ToolKind::Eraser => Tool::Eraser {
                radius: ui.default_eraser_radius,
            },
            ToolKind::Step => Tool::Step(StepSettings {
                fill: ui.default_stroke_color,
                text: Color::WHITE,
                radius: ui.default_step_radius,
                next_number: 1,
            }),
            ToolKind::Text => Tool::Text(crate::shape::TextStyle {
                color: ui.default_stroke_color,
                size: ui.default_text_size,
                bold: false,
            }),
        }
    }
}

/// Colours used by the built-in toolbar / popup / radial widgets.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, rename_all = "kebab-case"))]
pub struct ChromeColors {
    pub toolbar_bg: Color,
    pub toolbar_fg: Color,
    pub toolbar_border: Color,
    pub button_bg: Color,
    pub button_active_bg: Color,
    pub button_active_border: Color,
    pub accent: Color,
}

impl Default for ChromeColors {
    fn default() -> Self {
        Self {
            toolbar_bg: Color::rgb(22, 22, 24),
            toolbar_fg: Color::rgb(240, 240, 240),
            toolbar_border: Color::rgb(80, 80, 84),
            button_bg: Color::rgb(42, 42, 46),
            button_active_bg: Color::rgb(60, 110, 200),
            button_active_border: Color::rgb(180, 220, 255),
            accent: Color::ACCENT,
        }
    }
}

/// Host-configurable settings for the interactive overlay.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, rename_all = "kebab-case"))]
pub struct UiConfig {
    pub tools: Vec<ToolKind>,
    pub initial_tool: ToolKind,
    pub palette: Vec<Color>,
    pub radial_widths: Vec<f32>,
    pub default_stroke_color: Color,
    pub default_stroke_width: f32,
    pub default_fill: Option<Color>,
    pub default_blur_radius: f32,
    pub default_eraser_radius: f32,
    pub default_step_radius: f32,
    pub default_text_size: f32,
    pub snap_step: f32,
    pub region_outline_color: Color,
    /// Darken applied to pixels outside the active region (0..=255).
    pub background_dim: u8,
    pub chrome: ChromeColors,
    /// Initial state of the output-border toggle in the side action
    /// toolbar. Default `true` — the host passes `false` here when the
    /// user launched `sss` with `--no-border`, so the session opens with
    /// the toggle already off.
    pub border_enabled: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            tools: ToolKind::default_list(),
            initial_tool: ToolKind::Pointer,
            palette: Color::palette().to_vec(),
            radial_widths: vec![1.0, 3.0, 6.0, 12.0],
            default_stroke_color: Color::RED,
            default_stroke_width: 3.0,
            default_fill: None,
            default_blur_radius: 12.0,
            default_eraser_radius: 18.0,
            default_step_radius: 14.0,
            default_text_size: 18.0,
            snap_step: 10.0,
            region_outline_color: Color::WHITE,
            background_dim: 80,
            chrome: ChromeColors::default(),
            border_enabled: true,
        }
    }
}

impl UiConfig {
    /// Resolve `tools` + `initial_tool` into a `ToolPalette`.
    pub fn build_tool_palette(&self) -> ToolPalette {
        let mut tools: Vec<Tool> = self.tools.iter().copied().map(|k| k.build(self)).collect();
        if tools.is_empty() {
            tools.push(Tool::Pointer);
        }
        let initial = if self.tools.contains(&self.initial_tool) {
            self.initial_tool.build(self)
        } else {
            tools[0].clone()
        };
        ToolPalette {
            tools,
            color_palette: self.palette.clone(),
            initial,
        }
    }

    pub fn has_editor_tools(&self) -> bool {
        self.tools.iter().any(|k| *k != ToolKind::Pointer)
    }
}
