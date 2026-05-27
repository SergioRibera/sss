//! CPU `composite` finaliser and (feature-gated) GPU `overlay` preview.

pub mod composite;

#[cfg(feature = "editor")]
pub mod gpu;

#[cfg(feature = "editor")]
pub mod overlay;

// Cross-platform egui toolbar + radial menu + popups + magnifier. Used by
// the winit driver on every platform with the editor feature enabled.
#[cfg(feature = "editor")]
pub(crate) mod ui;
