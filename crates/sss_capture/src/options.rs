//! Capture-time options.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CaptureOptions {
    /// Whether the mouse cursor should be composited into the captured frame.
    /// Honoured by backends that support it (`wlr-screencopy`, the desktop
    /// portal, Win32 DXGI), silently ignored elsewhere.
    pub show_cursor: bool,
    /// Whether the backend should retry once on a transient failure. The
    /// most common case is a Wayland compositor returning `Failed` due to
    /// frame damage racing the request.
    pub retry_on_failure: bool,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            show_cursor: false,
            retry_on_failure: true,
        }
    }
}

impl CaptureOptions {
    pub const fn with_cursor() -> Self {
        Self {
            show_cursor: true,
            retry_on_failure: true,
        }
    }
}

/// Which backend implementation [`crate::Capturer`] should use.
///
/// `Auto` picks the best one available for the current platform at runtime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// Pick the best backend available at runtime.
    #[default]
    Auto,
    /// Linux native Wayland via `wlr-screencopy-unstable-v1`.
    Wayland,
    /// Linux Wayland via `org.freedesktop.portal.Screenshot` / `Screencast`.
    WaylandPortal,
    /// Linux X11 via x11rb (XGetImage / XShmGetImage + RANDR).
    X11,
    /// Windows Desktop GDI (`BitBlt`).
    WindowsGdi,
    /// Windows DXGI Desktop Duplication.
    WindowsDxgi,
    /// macOS CoreGraphics.
    MacOS,
}
