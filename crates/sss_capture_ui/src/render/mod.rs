//! `composite` bakes the canvas into the captured RGBA image (CPU, output to
//! PNG/clipboard). `overlay` paints the live editor on top of the desktop
//! through GPUI's `PathBuilder` API.

pub mod composite;
pub mod overlay;
