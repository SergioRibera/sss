//! Platform driver dispatch.

mod driver;

#[cfg(target_os = "linux")]
mod cursor;
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
            tracing::info!("sss_capture_ui: using wlr-layer-shell overlay driver");
            return wayland_layer::run(sel);
        }
    }
    tracing::info!("platform: routing to winit driver");
    driver::run(sel)
}
