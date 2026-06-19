+++
title = "Getting started"
description = "Render your first code screenshot in under a minute."
weight = 10
+++

## Install

Same install paths as `sss_cli` — see [install](/install/). Or from source:

```bash
cargo install --git https://github.com/SergioRibera/sss sss_code
```

## Your first render

```bash
sss_code src/main.rs > main.png
```

The default theme is `base16-ocean.dark`, default background `#323232`. Line numbers on, tab width 4.

## Pipe from stdin

```bash
cat src/main.rs | sss_code --extension rs > main.png
```

When piping you must hint the language with `-e <ext>` because the content has no filename.

## Pick a theme

```bash
sss_code --theme base16-monokai.dark src/main.rs > main.png
sss_code --list-themes        # show every embedded theme
sss_code --list-file-types    # show every supported language
```

## Highlight a code block

```bash
sss_code --lines 10..40 --highlight-lines 22..25 src/main.rs > snippet.png
```

`--lines` crops to that range. `--highlight-lines` keeps the cropped range but visually emphasises the inner range.

## Where it saves

Stdout — pipe to a file or to `wl-copy`/`xclip`:

```bash
sss_code src/main.rs | wl-copy --type image/png
```

## Next steps

- [Flags](/docs/code/flags/) — full CLI reference.
- [Themes](/docs/code/themes/) — built-in themes + how to import a vim color scheme.
- [Examples](/docs/code/examples/) — git diff renders, terminal sessions, multi-language galleries.
