+++
title = "sss_capture_ui"
description = "Library: build your own selector + annotator on top of the sss capture stack."
template = "section.html"
sort_by = "weight"
weight = 30
+++

`sss_capture_ui` is the library that powers the `sss-select` binary — and `sss_cli`'s interactive overlay. It's a Cargo crate you can depend on directly if you want to embed a region selector, window picker, or annotation canvas into your own Rust GUI.

<figure class="docs-figure">
  <video src="https://github.com/user-attachments/assets/db8949c9-dc17-4690-98e7-1b9acefd004c" autoplay loop muted playsinline></video>
  <figcaption>Selector overlay — pick a region, window or monitor, then annotate</figcaption>
</figure>

The crate is built on `winit` + `egui` + `wgpu` (Sergio's forks adding wlr-layer-shell support on Wayland) and re-exports the underlying `sss_capture` types so consumers don't need two dependencies.

## At a glance

- **`Selector` / `SelectorBuilder`** — main entry point. Spawn an overlay window, take a single selection, return an `Outcome`.
- **`UiConfig`** — toolbar layout, color palette, chrome colors, keybinds.
- **`ToolPalette` / `Tool` / `Shape` / `Style`** — annotation primitives.
- **`OcrPipeline`** — async pipeline trait you can plug your own OCR backend into.
- **`PostAction`** — whether the user hit "copy", "save", or chose a sub-action.

## When to use it

- You're building a screenshot app and want the same overlay as `sss`.
- You want the annotation canvas in your own UI without the capture stack.
- You want a custom OCR engine plugged into the same overlay.

The binary `sss-select` is a thin wrapper around `Selector::new(...).run()`. Read its source as the canonical "hello world."

## Next steps

- [Getting started](/docs/capture-ui/getting-started/) — minimal `Cargo.toml`, smallest selector.
- [API](/docs/capture-ui/api/) — public surface, types, callback contracts.
- [Integration](/docs/capture-ui/integration/) — driving the overlay from another Rust GUI / Tauri / a daemon.
- [Examples](/docs/capture-ui/examples/) — real consumer code from the workspace.
