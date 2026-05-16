//! # sss_capture_ui
//!
//! Interactive overlay for [`sss_capture`]. Provides:
//!
//! * A region / monitor / window **picker** (slurp-class, headless of toolbar).
//! * An optional **annotation editor** (brush, lines, arrows, shapes, blur,
//!   eraser, numbered "steps", text, plus a selection tool that re-edits
//!   anything already drawn).
//! * **Two capture timing modes**:
//!   * `CaptureTrigger::Eager` — take the screenshot up front, paint on top of
//!     it. Robust on every platform (the overlay is opaque pixels).
//!   * `CaptureTrigger::Lazy(...)` — show the overlay over the live desktop;
//!     the actual frame is grabbed when the user confirms (button or keybind).
//!
//! The library re-exports everything it needs from [`sss_capture`] so callers
//! don't have to depend on it directly.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations, rust_2018_idioms)]

pub use sss_capture::{
    self, Area, BackendKind, CaptureError, CaptureOptions, Capturer, Image, Monitor, MonitorId,
    Point, Rect, Rotation, Size, Window, WindowId, WindowSearch,
};

mod canvas;
mod color;
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
pub use mode::SelectorMode;
pub use selector::{Outcome, PostAction, Selection, Selector, SelectorBuilder, SelectorError};
pub use shape::{Shape, ShapeId, ShapeKind, Style, TextStyle};
pub use tool::{BrushSettings, StepSettings, Tool, ToolPalette};
pub use trigger::{CaptureTrigger, KeyBind, KeyChord};
