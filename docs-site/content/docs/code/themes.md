+++
title = "Themes"
description = "Bundled themes, vim color schemes, and custom .tmTheme imports."
weight = 30
+++

## Bundled themes

Every base16 theme that ships with `syntect` is embedded — no download needed. List them:

```bash
sss_code --list-themes
```

The default is `base16-ocean.dark`. Pick another with `--theme <name>`:

```bash
sss_code --theme base16-monokai.dark src/main.rs > out.png
sss_code --theme base16-github.light src/main.rs > out.png
```

## Custom `.tmTheme` files

Pass a path to `--theme` instead of a name:

```bash
sss_code --theme ~/themes/MyColors.tmTheme src/main.rs > out.png
```

The TextMate `.tmTheme` format is the same one Sublime Text, VS Code's Textmate theme converter, and bat use.

## From a vim color scheme

Map vim highlight groups to colors with `--vim-theme`. The format is `group,bg,fg,style;...`:

```bash
sss_code --vim-theme "Comment,,#7f8c8d,italic;String,,#a3be8c,;Function,,#88c0d0,bold" main.rs > out.png
```

Groups follow standard vim names (`Normal`, `Comment`, `String`, `Function`, `Type`, ...). Background and foreground accept hex (`#RRGGBB`) or empty for inherit. Style is comma-separated: `bold`, `italic`, `underline`, `reverse`.

This is convenient for matching your editor's exact colors without exporting a theme file.

## Build a cache

For large workloads, pre-bake the syntax + theme cache once:

```bash
sss_code --build-cache ~/.cache/sss
```

`sss_code` will pick up the cache automatically on subsequent runs. Useful in CI where the bundled assets cost startup time.

## Background tricks

Themes only control code colors. The background behind the code is controlled by `--code-background`:

```bash
# Solid color
sss_code --code-background "#1e1e2e" src/main.rs > out.png

# Horizontal gradient
sss_code --code-background "h;#1e1e2e;#313244" src/main.rs > out.png

# Vertical gradient
sss_code --code-background "v;#0f0f1a;#1e1e2e" src/main.rs > out.png

# Wallpaper image
sss_code --code-background ~/Pictures/wallpaper.jpg src/main.rs > out.png
```
