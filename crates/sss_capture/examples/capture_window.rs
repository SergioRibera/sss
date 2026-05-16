//! `cargo run -p sss_capture --example capture_window -- "Firefox" /tmp/window.png`
//!
//! Capture the first window whose title contains the given substring.

use sss_capture::{Capturer, Result};

fn main() -> Result<()> {
    let needle = std::env::args()
        .nth(1)
        .expect("usage: capture_window <title-substring> [out.png]");
    let out = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "/tmp/window.png".into());

    let cap = Capturer::new()?;
    let window = cap.window_by_title(&needle)?;
    println!("matched: {window}");

    cap.capture_window(&window)?.save(&out)?;
    println!("saved to {out}");
    Ok(())
}
