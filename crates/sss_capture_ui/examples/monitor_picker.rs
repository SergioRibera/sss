//! `cargo run -p sss_capture_ui --example monitor_picker`
//!
//! Click on any monitor to capture it.

use sss_capture_ui::{Outcome, SelectorBuilder, SelectorMode};

fn main() -> Result<(), sss_capture_ui::SelectorError> {
    let result = SelectorBuilder::default()
        .mode(SelectorMode::Monitor)
        .with_toolbar(false)
        .build()?
        .run()?;

    if let Outcome::Monitor {
        monitor,
        rect,
        image,
    } = result.outcome
    {
        println!("picked monitor {monitor} ({rect})");
        if let Some(img) = image {
            img.save("/tmp/sss_capture_ui_monitor.png")?;
        }
    } else {
        println!("cancelled");
    }
    Ok(())
}
