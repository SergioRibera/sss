//! `cargo run -p sss_capture --example select_backend -- [wayland|portal|x11|windows|macos|auto]`
//!
//! Build a [`Capturer`] forcing a specific backend, then print which one was
//! chosen and capture all monitors.

use sss_capture::{BackendKind, Capturer, Result};

fn main() -> Result<()> {
    let arg = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "auto".to_string());
    let kind = match arg.to_lowercase().as_str() {
        "wayland" => BackendKind::Wayland,
        "portal" | "wayland-portal" => BackendKind::WaylandPortal,
        "x11" => BackendKind::X11,
        "windows" | "windows-gdi" => BackendKind::WindowsGdi,
        "macos" => BackendKind::MacOS,
        _ => BackendKind::Auto,
    };

    let cap = Capturer::builder().backend(kind).build()?;
    println!("backend: {}", cap.backend_name());
    let img = cap.capture_all()?;
    let out = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "/tmp/backend.png".into());
    img.save(&out)?;
    println!("saved {}x{} to {}", img.width(), img.height(), out);
    Ok(())
}
