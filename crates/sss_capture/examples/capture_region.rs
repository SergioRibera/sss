//! `cargo run -p sss_capture --example capture_region -- 100,100 800x600 /tmp/region.png`
//!
//! Capture an arbitrary rectangle in logical desktop coordinates. The region
//! may span multiple monitors; `sss_capture` resolves the layout, captures
//! every overlapping display, rotates and rescales them, and stitches the
//! result into a single image whose top-left corresponds to the region's
//! top-left.

use sss_capture::{Capturer, Rect, Result};

fn main() -> Result<()> {
    let pos = std::env::args().nth(1).unwrap_or_else(|| "0,0".into());
    let size = std::env::args().nth(2).unwrap_or_else(|| "640x480".into());
    let out = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "/tmp/region.png".into());

    let (x, y) = pos
        .split_once(',')
        .map(|(a, b)| (a.parse::<i32>().unwrap_or(0), b.parse::<i32>().unwrap_or(0)))
        .unwrap_or_default();
    let (w, h) = size
        .split_once('x')
        .map(|(a, b)| {
            (
                a.parse::<u32>().unwrap_or(640),
                b.parse::<u32>().unwrap_or(480),
            )
        })
        .unwrap_or((640, 480));

    let region = Rect::from_xywh(x, y, w, h);
    println!("region: {region}");

    let cap = Capturer::new()?;
    let img = cap.capture_region(region)?;
    img.save(&out)?;
    println!("saved {}x{} to {}", img.width(), img.height(), out);
    Ok(())
}
