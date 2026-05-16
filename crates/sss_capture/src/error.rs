//! Error types returned by [`crate::Capturer`].

use thiserror::Error;

use crate::geometry::Rect;
use crate::{MonitorId, WindowId};

pub type Result<T, E = CaptureError> = std::result::Result<T, E>;

/// Every error variant `sss_capture` can return.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CaptureError {
    /// No display is connected (or no monitor is visible to the backend).
    #[error("no monitors are connected to the system")]
    NoMonitors,

    /// No window matched the request.
    #[error("no windows match the request")]
    NoWindows,

    #[error("monitor not found: {0}")]
    MonitorNotFound(MonitorId),

    #[error("window not found: {0}")]
    WindowNotFound(WindowId),

    #[error("point ({x}, {y}) does not fall inside any monitor")]
    PointOutsideDesktop { x: i32, y: i32 },

    #[error("region {0} does not overlap any monitor")]
    RegionOutsideDesktop(Rect),

    #[error("region has zero width or height: {0}")]
    EmptyRegion(Rect),

    #[error("cursor position is unavailable: {0}")]
    CursorUnavailable(String),

    /// Returned when neither the auto-selected backend nor any explicitly
    /// requested one could be initialised. The wrapped vector preserves the
    /// per-backend error message for diagnostics.
    #[error("no capture backend available: {}", .0.join("; "))]
    NoBackend(Vec<String>),

    /// Operation not supported by the backend (e.g. window capture on a
    /// pure-Wayland session without `ext-foreign-toplevel-list`).
    #[error("operation not supported by the {backend} backend: {detail}")]
    Unsupported {
        backend: &'static str,
        detail: String,
    },

    /// The operation timed out — typically `xdg-desktop-portal` taking too
    /// long to answer.
    #[error("backend timed out after {0:?}")]
    Timeout(std::time::Duration),

    /// User-cancelled (xdg-desktop-portal interactive picker).
    #[error("user cancelled the capture")]
    Cancelled,

    /// Catch-all for errors that bubble up from the platform backend.
    #[error("{backend}: {detail}")]
    Backend {
        backend: &'static str,
        detail: String,
    },

    /// I/O error from the platform.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Buffer ↔ image conversion failure (size mismatch, format unsupported).
    #[error("image conversion failed: {0}")]
    ImageConversion(String),
}

impl CaptureError {
    /// Build a `Backend` variant with a static backend tag.
    pub(crate) fn backend(backend: &'static str, detail: impl Into<String>) -> Self {
        CaptureError::Backend {
            backend,
            detail: detail.into(),
        }
    }

    /// Build an `Unsupported` variant.
    pub(crate) fn unsupported(backend: &'static str, detail: impl Into<String>) -> Self {
        CaptureError::Unsupported {
            backend,
            detail: detail.into(),
        }
    }
}
