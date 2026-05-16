//! Linux backends: native Wayland (wlr-screencopy), xdg-desktop-portal, X11.

use std::env;

pub(crate) mod portal;
pub(crate) mod wayland;
pub(crate) mod x11;

/// Returns true when the current process is talking to a Wayland compositor.
pub(crate) fn is_wayland_session() -> bool {
    let xdg = env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let display = env::var("WAYLAND_DISPLAY").unwrap_or_default();
    xdg.eq_ignore_ascii_case("wayland") || !display.is_empty()
}

/// Returns true when an X server is reachable through `$DISPLAY`. On Wayland
/// sessions this is XWayland, which is still useful (cursor queries, window
/// captures) when the compositor lacks the needed protocols.
pub(crate) fn is_x11_session() -> bool {
    !env::var("DISPLAY").unwrap_or_default().is_empty()
}
