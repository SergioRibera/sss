+++
title = "Examples"
description = "Real consumer code from the workspace and beyond."
weight = 40
+++

## Minimal `sss-select`

The `sss-select` binary ships with the crate. Source:

```rust
use sss_capture_ui::{Selector, SelectorMode, UiConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let outcome = Selector::builder()
        .mode(SelectorMode::AnyOf(&[
            SelectorMode::Area,
            SelectorMode::Window,
            SelectorMode::Monitor,
        ]))
        .ui_config(UiConfig::default())
        .build()?
        .run()?;

    match outcome.post_action {
        sss_capture_ui::PostAction::Copy     => outcome.write_to_clipboard()?,
        sss_capture_ui::PostAction::Save(p)  => outcome.write_png(&p)?,
        sss_capture_ui::PostAction::Cancelled => {}
    }
    Ok(())
}
```

## With a custom palette + tools

```rust
use sss_capture_ui::{Color, ToolKind, ToolPalette, UiConfig};

let mut ui = UiConfig::default();
ui.palette = vec![
    Color::hex("#ff4f8b"),
    Color::hex("#7c3aed"),
    Color::hex("#06b6d4"),
    Color::hex("#fcd34d"),
];
ui.default_tool = ToolKind::Arrow;
ui.toolbar = true;
```

## Streaming OCR onto the overlay

```rust
use sss_capture_ui::{OcrPipeline, Image, Selector};
use std::sync::mpsc;

fn build_pipeline() -> OcrPipeline {
    Box::new(|image: Image| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for tb in my_ocr::recognize(image.as_rgba_bytes()) {
                let _ = tx.send(tb);
            }
        });
        rx
    })
}

let outcome = Selector::builder()
    .ocr_pipeline(build_pipeline())
    .build()?
    .run()?;
```

## Re-render annotations with your own painter

```rust
for shape in &outcome.shapes {
    match &shape.kind {
        ShapeKind::Arrow         => draw_arrow(&shape.style, shape.points()),
        ShapeKind::Rectangle     => draw_rect(&shape.style, shape.bounds()),
        ShapeKind::Text(text)    => draw_text(&shape.style, text),
        ShapeKind::Blur          => apply_blur(shape.bounds()),
        ShapeKind::Freehand      => draw_path(&shape.style, shape.points()),
        ShapeKind::Step(n)       => draw_step(*n, shape.center()),
    }
}
```

Shapes carry world-space coordinates relative to the captured image — they don't depend on the overlay's window position.

## Pure picker (no annotation surface)

Disable the editor feature in `Cargo.toml`:

```toml
sss_capture_ui = {
  git = "https://github.com/SergioRibera/sss",
  default-features = false,
}
```

Then:

```rust
let outcome = Selector::builder()
    .mode(SelectorMode::Window)
    .build()?
    .run()?;
```

The overlay is now selector-only — no toolbar, no shapes, ~40% smaller binary.
