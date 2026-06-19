+++
title = "sss_cli"
description = "Interactive screen + window + region capture with annotation overlay."
template = "section.html"
sort_by = "weight"
weight = 10
+++

`sss_cli` (binary: `sss`) is the flagship screenshot tool. Pick a region, a window, or a monitor; annotate it; save or copy. Wayland and X11 first-class; macOS via the native ScreenCaptureKit path.

## At a glance

- Three target modes: **screen**, **window**, **area** — each with an interactive picker or a direct selector.
- Annotation overlay: shapes, arrows, text, pen, blur, pipette.
- Optional **OCR** layer (`oar-ocr`) to copy text directly from images.
- Compositing options: cursor on/off, decoration on/off, border on/off.
- Backends auto-detected: `wayland`, `portal`, `x11`, `windows`, `macos`.

## When to use it

| Goal | Flag combination |
| --- | --- |
| Region select with editor | `sss --area` |
| Window picker | `sss --window` |
| Whole screen, no UI | `sss --screen --current` |
| Force portal backend | `sss --capture-backend portal` |
| Disable OCR | `sss --ocr=false` |

Walk through the [getting started](/docs/cli/getting-started/) guide next, or jump to [flags](/docs/cli/flags/) for the full reference.
