//! Capture-time options.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CaptureOptions {
    /// Composite the mouse cursor into the captured frame when supported.
    pub show_cursor: bool,
    /// Retry once on a transient failure.
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BackendKind {
    #[default]
    Auto,
    /// ext-image-copy-capture-v1 (cosmic, future GNOME/KWin/sway).
    WaylandExt,
    /// zwlr_screencopy_manager_v1 (wlroots: sway/Hyprland/niri/river).
    Wayland,
    WaylandPortal,
    X11,
    WindowsGdi,
    WindowsDxgi,
    MacOS,
}
