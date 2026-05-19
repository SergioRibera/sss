//! `cargo run -p sss_capture_ui --example select_region`
//!
//! Minimal slurp-class flow: opens the overlay, lets the user drag a
//! rectangle, prints it.

use sss_capture_ui::{Outcome, SelectorBuilder, SelectorMode};

fn main() -> Result<(), sss_capture_ui::SelectorError> {
    let result = SelectorBuilder::default()
        .mode(SelectorMode::Area)
        .with_toolbar(false)
        .build()?
        .run()?;

    match result.outcome {
        Outcome::Region { rect, image } => {
            println!("region: {rect}");
            if let Some(img) = image {
                let _ = img.save("/tmp/sss_capture_ui_region.png");
            }
        }
        Outcome::Cancelled => println!("cancelled"),
        other => println!("{other:?}"),
    }
    Ok(())
}
