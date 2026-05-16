//! Rendering — two distinct paths:
//!
//! * `composite` (always available, CPU-only): rasterises every shape onto
//!   a captured `RgbaImage`, applies blur masks, returns the final image.
//!   This is what produces the screenshot the user actually saves.
//! * `overlay` (under `feature = "editor"`): GPU-accelerated egui-based
//!   overlay rendering. Drives the interactive preview while the user is
//!   still editing.

pub mod composite;

#[cfg(feature = "editor")]
pub mod gpu;

#[cfg(feature = "editor")]
pub mod overlay;
