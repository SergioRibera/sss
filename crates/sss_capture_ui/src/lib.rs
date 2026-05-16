//! Interactive selector and annotation overlay for [`sss_capture`].

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations, rust_2018_idioms)]

pub use sss_capture::{
    self, Area, BackendKind, CaptureError, CaptureOptions, Capturer, Image, Monitor, MonitorId,
    Point, Rect, Rotation, Size, Window, WindowId, WindowSearch,
};

mod canvas;
mod color;
mod config;
mod geometry;
mod hit;
mod mode;
mod selector;
mod shape;
mod tool;
mod trigger;

mod render;

mod platform;

pub use canvas::Canvas;
pub use color::Color;
pub use config::{ChromeColors, ToolKind, UiConfig};
pub use mode::SelectorMode;
pub use selector::{Outcome, PostAction, Selection, Selector, SelectorBuilder, SelectorError};
pub use shape::{Shape, ShapeId, ShapeKind, Style, TextStyle};
pub use tool::{BrushSettings, StepSettings, Tool, ToolPalette};
pub use trigger::{CaptureTrigger, KeyBind, KeyChord};
