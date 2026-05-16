//! Cross-platform screen capture for Super ScreenShot.
//!
//! ```no_run
//! use sss_capture::{Capturer, Rect};
//!
//! # fn main() -> Result<(), sss_capture::CaptureError> {
//! let cap = Capturer::new()?;
//! cap.capture_all()?.save("/tmp/desktop.png")?;
//! cap.capture_at_cursor()?.save("/tmp/here.png")?;
//! cap.capture_region(Rect::from_xywh(0, 0, 1920, 1080))?
//!    .save("/tmp/region.png")?;
//! # Ok(()) }
//! ```

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations, rust_2018_idioms)]

mod backend;
mod capturer;
mod error;
mod frame;
mod geometry;
mod monitor;
mod options;
mod window;

pub use ::image;

pub use capturer::{Capturer, CapturerBuilder};
pub use error::{CaptureError, Result};
pub use frame::Image;
pub use geometry::{Area, Point, Rect, Rotation, Size};
pub use monitor::{Monitor, MonitorId};
pub use options::{BackendKind, CaptureOptions};
pub use window::{Window, WindowId, WindowSearch};
