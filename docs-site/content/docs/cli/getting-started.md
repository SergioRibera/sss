+++
title = "Getting started"
description = "Install sss, take your first annotated screenshot in under a minute."
weight = 10
+++

## Install

Pick your platform on the [install page](/install/), or use Cargo from source:

```bash
cargo install --git https://github.com/SergioRibera/sss sss_cli
```

## Your first capture

Open an interactive area selector with the annotation overlay:

```bash
sss --area
```

Click-drag to pick a region. Tools appear in the toolbar — pen, rectangle, arrow, text, blur, pipette. Hit <kbd>Enter</kbd> to confirm, <kbd>Esc</kbd> to cancel.

## Pick a window instead

```bash
sss --window
```

Hover any window to highlight it; click to select. Add `--window <title>` to skip the picker and target a window by title substring.

## Capture the current monitor, headless

```bash
sss --screen --current
```

`--current` means "the monitor under the mouse cursor right now." Pair with `--show-cursor` to composite the cursor into the frame.

## Where it saves

By default `sss` copies the result to your clipboard and exits. Use `--output <path>` to save a PNG to disk instead.

## Config file

Create `~/.config/sss/config.toml` to set defaults. Example:

```toml
[general]
output = "~/Pictures/Screenshots/sss-%Y-%m-%d_%H-%M-%S.png"
show-cursor = true

[capture-ui]
toolbar = true
remember-last-selection = true

[ocr]
enabled = true
gpu = "auto"
```

See the [config reference](/docs/config-reference/) for every key. CLI flags always override config values.

## Next steps

- [Flags](/docs/cli/flags/) — the full CLI reference.
- [Configuration](/docs/cli/configuration/) — TOML layout and import semantics.
- [Examples](/docs/cli/examples/) — real workflows.
