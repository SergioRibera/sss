//! `cargo run -p sss_capture_ui --example annotate --features editor`
//!
//! Full editor flow with toolbar. Lets the user pick a region and decorate
//! it with brush strokes, arrows, blur rectangles, step markers and text.

use sss_capture_ui::{CaptureTrigger, Outcome, SelectorBuilder, SelectorMode, ToolPalette};

fn main() -> Result<(), sss_capture_ui::SelectorError> {
    let result = SelectorBuilder::new()
        .mode(SelectorMode::AnyOf)
        .with_toolbar(true)
        .palette(ToolPalette::default())
        .capture_trigger(CaptureTrigger::Eager)
        .build()?
        .run()?;

    match result.outcome {
        Outcome::Region { rect, image } => {
            println!(
                "region {rect} with {} shape(s)",
                result.canvas.shapes().len()
            );
            if let Some(img) = image {
                img.save("/tmp/sss_capture_ui_annotated.png")?;
            }
        }
        Outcome::Cancelled => println!("cancelled"),
        o => println!("{o:?}"),
    }
    Ok(())
}
