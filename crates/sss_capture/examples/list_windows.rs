//! `cargo run -p sss_capture --example list_windows`
//!
//! Enumerate top-level windows. On a pure Wayland session without a portal,
//! this prints an empty list — windows aren't visible to apps.

use sss_capture::{Capturer, Result};

fn main() -> Result<()> {
    let cap = Capturer::new()?;
    let windows = cap.windows()?;
    if windows.is_empty() {
        println!("no windows visible to {}", cap.backend_name());
        return Ok(());
    }
    for w in windows {
        println!(
            "  {id}  {app:<24} {title:<40} {bounds}{flags}",
            id = w.id(),
            app = w.app_name(),
            title = w.title(),
            bounds = w.bounds(),
            flags = match (w.is_minimized(), w.is_maximized()) {
                (true, _) => "  [min]",
                (_, true) => "  [max]",
                _ => "",
            },
        );
    }
    Ok(())
}
