//! CPU `composite` finaliser and (feature-gated) GPU `overlay` preview.

pub mod composite;

#[cfg(feature = "editor")]
pub mod gpu;

#[cfg(feature = "editor")]
pub mod overlay;
