# sss_capture_ui

Interactive selection / annotation overlay built on top of
[`sss_capture`](../sss_capture).

Three things in one crate:

1. **Region / monitor / window picker** — slurp-class flow. Drag a
   rectangle, click a monitor, click a window. Optional toolbar.
2. **Annotation editor** — toolbar with brush, line, arrow, rectangle,
   ellipse, blur rectangle, eraser, numbered "steps" and text. Every
   committed shape stays editable through the Pointer tool.
3. **`sss-select` binary** — a drop-in replacement for `slurp`. Prints
   `x,y WxH` to stdout; supports `--monitor`, `--window`, `--save out.png`.

The annotation layer is gated behind the `editor` feature so the slurp-class
flow can compile without pulling in egui / wgpu.

---

## Capture timing

```rust
pub enum CaptureTrigger {
    /// Default. Capture the desktop up front; user paints over the static image.
    Eager,
    /// Show overlay over the live desktop; capture happens when the user confirms.
    Lazy { confirm: KeyChord, confirm_button_label: Option<String> },
}
```

Eager mode is rock-solid on every platform (overlay can be opaque pixels).
Lazy mode needs real compositor transparency and works best on X11 /
wlroots / Win32 / macOS — on GNOME / KDE Wayland it can flicker.

---

## Selector modes

```rust
pub enum SelectorMode {
    Area,      // drag a rectangle
    Monitor,   // click a monitor
    Window,    // click a window; previews float over the monitor that hosts them
    AnyOf,     // user toggles between the three through toolbar tabs (default with toolbar)
}
```

For **Window mode** the selector draws a thumbnail of every visible
top-level window on top of its host monitor — that's why the picker can
target windows that are partially or fully occluded.

---

## Tools

```rust
pub enum Tool {
    Pointer,                 // select / move / resize / restyle existing shapes
    Brush(BrushSettings),    // freehand paint with color + width
    Line(BrushSettings),
    Arrow(BrushSettings),
    Rectangle(BrushSettings),
    Ellipse(BrushSettings),
    BlurRect { radius: f32 },
    Eraser { radius: f32 },
    Step(StepSettings),      // numbered circles for step-by-step screenshots
    Text(TextStyle),
}
```

Every shape is editable post-hoc — pick the Pointer, click a shape, drag
to move, change its color through the palette.

The Pointer tool also drives the **region rectangle** itself: dragging
empty space defines / resizes the selection.

---

## Quick start

```rust
use sss_capture_ui::{Outcome, SelectorBuilder, SelectorMode, CaptureTrigger};

let result = SelectorBuilder::default()
    .mode(SelectorMode::Area)
    .with_toolbar(true)
    .capture_trigger(CaptureTrigger::Eager)
    .build()?
    .run()?;

match result.outcome {
    Outcome::Region { rect, image } => {
        println!("captured {rect}");
        if let Some(img) = image { img.save("out.png")?; }
    }
    Outcome::Monitor { monitor, .. } => println!("monitor {monitor}"),
    Outcome::Window  { window,  .. } => println!("window {window}"),
    Outcome::Cancelled => println!("escape"),
}
```

---

## Cursor on Wayland — the trick

The protocol intentionally hides the global pointer position. The
selector works around it the same way `slurp` does: by **owning** the
overlay surface across every output. Once your surface has pointer focus
the compositor sends every `motion` event with surface-local
coordinates; we translate to global by adding the surface's monitor
origin. Selection across cuts between monitors keeps working because
`leave` on one output is paired with `enter` on the next.

---

## Library structure

```
src/
├── lib.rs              public re-exports
├── selector.rs         Selector / Builder / Outcome / Selection / SelectorError
├── mode.rs             SelectorMode enum
├── trigger.rs          CaptureTrigger + KeyChord
├── tool.rs             Tool enum + BrushSettings / StepSettings / ToolPalette
├── shape.rs            Shape / ShapeKind / ShapeId / Style / TextStyle
├── canvas.rs           Canvas (state machine: drag, shapes, history, region)
├── hit.rs              shape hit-testing for the Pointer tool
├── color.rs            Color primitive + default palette
├── geometry.rs         FPoint / FRect (sub-pixel editing helpers)
├── render/
│   ├── composite.rs    CPU-only flatten: bakes shapes onto the captured RGBA
│   └── overlay.rs      (feature = "editor") egui-based interactive overlay
├── platform/
│   └── driver.rs       winit-based event loop, one fullscreen window per output
└── bin/sss_select.rs   slurp-equivalent CLI
```

---

## Examples

```bash
cargo run -p sss_capture_ui --example select_region
cargo run -p sss_capture_ui --example monitor_picker
cargo run -p sss_capture_ui --example annotate --features editor

# slurp-class binary:
cargo run -p sss_capture_ui --bin sss-select -- --area
cargo run -p sss_capture_ui --bin sss-select -- --monitor --save mon.png
```

---

## Status

| Feature                                                       | State |
| ------------------------------------------------------------- | ----- |
| Cross-platform overlay (X11 / Wayland xdg / Win / macOS)      | ✓     |
| Region rubber-band + Escape / Enter                           | ✓     |
| Monitor picker                                                | ✓     |
| Window picker (basic; thumbnails behind `editor` feature)     | ◐     |
| Canvas state machine with full Tool / Shape / undo model      | ✓     |
| CPU compositor (shapes + Gaussian blur)                       | ✓     |
| egui toolbar overlay                                          | ◐ (scaffolded; wgpu wiring in progress) |
| Wayland `wlr-layer-shell` z-order above panels                | ◯ (xdg-shell fullscreen works everywhere as a baseline) |
| GNOME / KDE portal interactive picker fallback                | ◯     |

The CPU compositor and canvas state machine are fully exercised by unit
tests and the `select_region` / `monitor_picker` examples; the egui
overlay path is wired through the `editor` feature and gets enabled
progressively as we tighten platform integration.

---

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE), matching the rest of the workspace.
