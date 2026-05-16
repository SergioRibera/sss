//! `cargo run -p sss_capture --example capture_primary -- /tmp/primary.png`
//!
//! Capture only the primary monitor.

use std::path::PathBuf;

use sss_capture::{Capturer, Result};

fn main() -> Result<()> {
    let out: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/primary.png".to_string())
        .into();

    let cap = Capturer::new()?;
    let primary = cap.primary_monitor()?;
    println!("primary: {primary}");

    let img = cap.capture_monitor(&primary)?;
    img.save(&out)?;
    println!("saved to {}", out.display());
    Ok(())
}
