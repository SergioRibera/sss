//! # sss_capture
//!
//! Cross-platform screen capture for [Super ScreenShot][repo], implemented
//! from scratch on top of native OS bindings — **no third-party capture
//! library is involved**. On every platform we talk directly to the kernel /
//! windowing system through the canonical low-level crates:
//!
//! | Platform           | Backend                                    |
//! | ------------------ | ------------------------------------------ |
//! | Wayland (wlroots)  | `wlr-screencopy-unstable-v1` via wayland-client |
//! | Wayland (GNOME/KDE)| `org.freedesktop.portal.Screenshot` via zbus    |
//! | Linux X11          | x11rb (XGetImage + RANDR + EWMH + XFixes)       |
//! | Windows            | Win32 (GDI BitBlt + EnumDisplayMonitors + …)   |
//! | macOS              | CoreGraphics (CGDisplayCreateImage + …)         |
//!
//! [repo]: https://github.com/SergioRibera/sss
//!
//! ## Quick start
//!
//! ```no_run
//! use sss_capture::{Capturer, Rect};
//!
//! # fn main() -> Result<(), sss_capture::CaptureError> {
//! let cap = Capturer::new()?;                       // auto-detected backend
//! cap.capture_all()?.save("/tmp/desktop.png")?;     // full virtual desktop
//! cap.capture_at_cursor()?.save("/tmp/here.png")?;  // monitor under cursor
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
