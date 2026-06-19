+++
title = "Getting started"
description = "Add sss_capture_ui to a Rust project and spawn a selector in under 20 lines."
weight = 10
+++

## Add to your project

```toml
# Cargo.toml
[dependencies]
sss_capture_ui = { git = "https://github.com/SergioRibera/sss" }
```

Crates.io publishing is pending; pin to a tag once available.

## Spawn a region selector

```rust
use sss_capture_ui::{Selector, SelectorMode, UiConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let outcome = Selector::builder()
        .mode(SelectorMode::Area)
        .ui_config(UiConfig::default())
        .build()?
        .run()?;

    match outcome.post_action {
        sss_capture_ui::PostAction::Copy => {
            outcome.write_to_clipboard()?;
        }
        sss_capture_ui::PostAction::Save(path) => {
            outcome.write_png(&path)?;
        }
        sss_capture_ui::PostAction::Cancelled => {}
    }
    Ok(())
}
```

That's the same code path `sss-select` runs. The overlay opens, the user picks a region, annotates if they want, and the `Outcome` carries the pixels + the user's intent.

## Picker modes

```rust
SelectorMode::Area      // free region (default)
SelectorMode::Window    // click to pick a window
SelectorMode::Monitor   // click to pick a monitor
SelectorMode::AnyOf(&[Area, Window])   // composite, switchable from toolbar
```

## Feature flags

```toml
sss_capture_ui = {
  git = "https://github.com/SergioRibera/sss",
  default-features = false,
  features = ["editor", "serde"],
}
```

- **`editor`** (default) — pulls in egui + wgpu for the annotation canvas.
- **`serde`** — derive Serialize/Deserialize on config types.

Disable `editor` if you only want a picker without the annotation surface.

## Next

- [API](/docs/capture-ui/api/) — every public type.
- [Integration](/docs/capture-ui/integration/) — running from another GUI thread / async runtime.
