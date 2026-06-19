+++
title = "API"
description = "Public surface of sss_capture_ui: Selector, UiConfig, ToolPalette, Outcome."
weight = 20
+++

## Top-level

### `Selector` / `SelectorBuilder`

The entry point. Build with the builder, run once, get an `Outcome`.

```rust
let outcome = Selector::builder()
    .mode(SelectorMode::Area)
    .ui_config(UiConfig::default())
    .ocr_pipeline(my_pipeline)        // optional
    .text_clipboard(my_clipboard)     // optional
    .build()?
    .run()?;
```

### `Outcome`

```rust
pub struct Outcome {
    pub selection: Selection,
    pub post_action: PostAction,
    pub image: Image,
    /// Annotations the user drew, in their original drawing order.
    pub shapes: Vec<Shape>,
    /// Whether the user toggled the chrome/border on or off.
    pub border: bool,
}
```

Helper methods on `Outcome`:

- `write_to_clipboard()` — push the composited PNG to the system clipboard.
- `write_png(&path)` — encode + save.
- `into_image()` — strip metadata, keep the raster.

### `PostAction`

```rust
pub enum PostAction {
    Copy,
    Save(PathBuf),
    Cancelled,
}
```

## Modes

```rust
pub enum SelectorMode {
    Area,
    Window,
    Monitor,
    AnyOf(&'static [SelectorMode]),
}
```

`AnyOf` puts a mode-switcher in the toolbar.

## Configuration

### `UiConfig`

```rust
pub struct UiConfig {
    pub toolbar: bool,
    pub palette: ColorPalette,
    pub chrome: ChromeColors,
    pub default_tool: ToolKind,
    pub remember_last_selection: bool,
}
```

Mirrors `[capture-ui]` in `~/.config/sss/config.toml`. See the [config reference](/docs/config-reference/) for keys.

### `ChromeColors`

Hex strings for selector border, dim overlay, handles, hover ring.

## Annotation primitives

```rust
pub struct Shape  { pub kind: ShapeKind, pub style: Style, /* ... */ }
pub enum ShapeKind { Freehand, Rectangle, Arrow, Text(String), Blur, Step(u32) }
pub struct Style  { pub color: Color, pub stroke: f32, pub fill: Option<Color> }
pub struct Tool   { pub kind: ToolKind, pub brush: BrushSettings, /* ... */ }
pub enum ToolKind { Pen, Eraser, Rect, Arrow, Text, Blur, Pipette, Step }
```

Shapes are emitted in `Outcome.shapes` in drawing order. You can re-render them onto your own canvas if you want a non-PNG output.

## OCR integration

```rust
pub type OcrPipeline = Box<
    dyn Fn(Image) -> mpsc::Receiver<TextBox> + Send + Sync,
>;
```

Plug your own OCR backend in by implementing the closure. `sss_cli` wires the `sss_ocr` crate; you can wire anything that emits `TextBox`es over a channel.

## Re-exports from `sss_capture`

The crate re-exports the capture types so downstream users don't need a second dependency:

```rust
pub use sss_capture::{Area, BackendKind, CaptureError, Capturer, Image, Monitor, Point, Rect, Size, Window};
```
