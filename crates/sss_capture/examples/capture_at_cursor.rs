//! `cargo run -p sss_capture --example capture_at_cursor -- /tmp/cursor.png`
//!
//! Capture the monitor that currently contains the mouse cursor.
//!
//! On Wayland the compositor may refuse to report the pointer location —
//! this example handles that gracefully by falling back to the primary
//! monitor.

use std::path::PathBuf;

use sss_capture::{CaptureError, Capturer, Result};

fn main() -> Result<()> {
    let out: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/cursor.png".to_string())
        .into();

    let cap = Capturer::new()?;

    let img = match cap.capture_at_cursor() {
        Ok(img) => {
            println!("captured monitor under the cursor");
            img
        }
        Err(CaptureError::CursorUnavailable(msg)) => {
            eprintln!("cursor unavailable ({msg}); using primary monitor instead");
            let primary = cap.primary_monitor()?;
            cap.capture_monitor(&primary)?
        }
        Err(e) => return Err(e),
    };

    img.save(&out)?;
    println!("saved to {}", out.display());
    Ok(())
}
