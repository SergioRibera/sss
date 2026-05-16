//! `cargo run -p sss_capture --example capture_all -- /tmp/desktop.png`
//!
//! Capture the entire virtual desktop and save it to disk.

use std::path::PathBuf;

use sss_capture::{Capturer, Result};

fn main() -> Result<()> {
    let out: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/desktop.png".to_string())
        .into();

    let cap = Capturer::new()?;
    println!("backend: {}", cap.backend_name());

    let img = cap.capture_all()?;
    println!("captured {}x{}", img.width(), img.height());

    img.save(&out)?;
    println!("saved to {}", out.display());
    Ok(())
}
