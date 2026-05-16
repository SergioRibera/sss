//! `cargo run -p sss_capture --example list_monitors`
//!
//! Prints every monitor the platform reports. Useful for figuring out the
//! `id`s and `name`s you can pass to `Capturer::monitor_by_*`.

use sss_capture::{Capturer, Result};

fn main() -> Result<()> {
    let cap = Capturer::new()?;
    println!("backend: {}", cap.backend_name());
    for m in cap.monitors()? {
        let hz = m
            .refresh_rate()
            .map(|h| format!("{h:.0}Hz"))
            .unwrap_or_else(|| "?Hz".to_string());
        println!(
            "  {id}  {name:<24} {bounds}  scale {scale:.2}x  rot {rot:?}  {hz}{primary}",
            id = m.id(),
            name = m.name(),
            bounds = m.bounds(),
            scale = m.scale_factor(),
            rot = m.rotation(),
            primary = if m.is_primary() { "  (primary)" } else { "" },
        );
    }
    Ok(())
}
