//! Error types returned by [`crate::Capturer`].

use thiserror::Error;

use crate::geometry::Rect;
use crate::{MonitorId, WindowId};

pub type Result<T, E = CaptureError> = std::result::Result<T, E>;

/// Error variants returned by `sss_capture`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CaptureError {
    #[error("no monitors are connected to the system")]
    NoMonitors,

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

    /// Returned when no capture backend could be initialised.
    #[error("no capture backend available: {}", .0.join("; "))]
    NoBackend(Vec<String>),

    /// Operation not supported by the backend.
    #[error("operation not supported by the {backend} backend: {detail}")]
    Unsupported {
        backend: &'static str,
        detail: String,
    },

    #[error("backend timed out after {0:?}")]
    Timeout(std::time::Duration),

    #[error("user cancelled the capture")]
    Cancelled,

    #[error("{backend}: {detail}")]
    Backend {
        backend: &'static str,
        detail: String,
    },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("image conversion failed: {0}")]
    ImageConversion(String),
}

impl CaptureError {
    pub(crate) fn backend(backend: &'static str, detail: impl Into<String>) -> Self {
        CaptureError::Backend {
            backend,
            detail: detail.into(),
        }
    }

    pub(crate) fn unsupported(backend: &'static str, detail: impl Into<String>) -> Self {
        CaptureError::Unsupported {
            backend,
            detail: detail.into(),
        }
    }
}
