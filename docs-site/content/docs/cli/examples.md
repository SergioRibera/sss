+++
title = "Examples"
description = "Real-world sss invocations: region capture, window picker, OCR copy, scripted clipboard."
weight = 40
+++

## Pick a region and save to dated file

```bash
sss --area --output ~/Pictures/Screenshots/sss-$(date +%F_%H-%M-%S).png
```

## Window picker into clipboard, no toolbar

```bash
sss --window --no-toolbar
```

## Whole current monitor, with cursor, copied

```bash
sss --screen --current --show-cursor
```

## Force the Wayland backend (skip portal)

```bash
sss --area --capture-backend wayland
```

Useful when running under wlroots compositors (sway, river, hyprland) where the Wayland backend is more reliable than the portal one.

## OCR copy: capture, extract text

```bash
sss --area --ocr
```

After selecting the region, the OCR pass highlights detected text. Click any line to copy just that text to the clipboard.

## Disable OCR globally

```toml
# ~/.config/sss/config.toml
[ocr]
enabled = false
```

Or per-call:

```bash
sss --area --ocr=false
```

## Bind it to a global hotkey

### Sway (`~/.config/sway/config`)

```
bindsym Print              exec sss --area
bindsym Shift+Print        exec sss --window
bindsym Ctrl+Print         exec sss --screen --current
```

### Hyprland (`~/.config/hypr/hyprland.conf`)

```
bind = , Print,       exec, sss --area
bind = SHIFT, Print,  exec, sss --window
bind = CTRL, Print,   exec, sss --screen --current
```

### GNOME / KDE

Use the system shortcuts UI; point them at `sss --area` (etc.) directly.

## Pipe into ImageMagick

`sss` writes PNG to stdout if `--output -` is set. Pair with `magick` for post-processing:

```bash
sss --area --output - | magick - -quality 85 ~/Pictures/sss.jpg
```
