+++
title = "sss_cli"
description = "Interactive screen + window + region capture with annotation overlay."
template = "section.html"
sort_by = "weight"
weight = 10
+++

`sss_cli` (binary: `sss`) is the flagship screenshot tool. Pick a region, a window, or a monitor; annotate it; save or copy. Wayland and X11 first-class; macOS via the native ScreenCaptureKit path.

<figure class="docs-figure">
  <img src="https://github.com/SergioRibera/sss/assets/56278796/945f224c-96ec-48b6-a738-50ac2c9cfb90" alt="sss_cli annotation overlay — arrow, shapes and blur on a captured region" loading="lazy">
  <figcaption>Annotation overlay — arrows, shapes, blur, text</figcaption>
</figure>

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

## OCR

The annotation overlay can pipe the captured (or selected) region through an OCR engine and hand the recognized text straight back to the clipboard. Powered by [`sss_ocr`](https://github.com/SergioRibera/sss/tree/main/crates/sss_ocr) with hardware-aware defaults (CPU / CUDA / TensorRT / CoreML / DirectML / OpenVINO / WebGPU).

<figure class="docs-figure">
  <img src="https://github.com/user-attachments/assets/96be5220-2ab7-445f-9ea8-5f7c29944ac1" alt="sss_cli OCR extracting text from a captured region" loading="lazy">
  <figcaption>OCR — select, recognize, copy</figcaption>
</figure>

Disable with `--ocr=false` or pin a backend with `--ocr-ep cuda|tensorrt|coreml|directml|openvino|webgpu`.

Walk through the [getting started](/docs/cli/getting-started/) guide next, or jump to [flags](/docs/cli/flags/) for the full reference.
