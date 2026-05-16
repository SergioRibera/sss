//! Platform driver dispatch.
//!
//! Two paths today:
//!
//! * [`wayland_layer`] — native `wlr-layer-shell` + CPU rendering. This is
//!   the only path that actually works on tiling Wayland compositors
//!   (niri, sway, Hyprland, river, cosmic): the overlay sits on the
//!   *Overlay* layer above every other client and the rendering goes
//!   through `wl_shm` so the GPU driver isn't asked to composite four
//!   translucent surfaces in real time. No wgpu / no winit.
//! * [`driver`] — `winit` + `wgpu` + `egui`. The fallback for X11, Win32,
//!   macOS, and Wayland sessions that don't advertise `zwlr_layer_shell_v1`
//!   (GNOME, KDE).

mod driver;

#[cfg(target_os = "linux")]
mod cursor;
#[cfg(target_os = "linux")]
mod font;
#[cfg(target_os = "linux")]
mod icons;
#[cfg(target_os = "linux")]
mod wayland_layer;

pub(crate) fn run(
    sel: crate::selector::Selector,
) -> Result<crate::selector::Selection, crate::selector::SelectorError> {
    #[cfg(target_os = "linux")]
    {
        if wayland_layer::is_available() {
            tracing::info!("platform: routing to wayland layer-shell driver");
            eprintln!("sss_capture_ui: using wlr-layer-shell overlay driver");
            return wayland_layer::run(sel);
        }
    }
    tracing::info!("platform: routing to winit driver");
    driver::run(sel)
}
