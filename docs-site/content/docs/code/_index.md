+++
title = "sss_code"
description = "Render source files into pretty PNG screenshots with syntax highlighting."
template = "section.html"
sort_by = "weight"
weight = 20
+++

`sss_code` is a CLI-only renderer: feed it a source file or `stdin`, get back a PNG with syntax highlighting, line numbers, gradients, and optional macOS-style window chrome.

It bundles every `syntect` language and theme out of the box, so you can render Rust, TS, Go, Python, Nix, Markdown — anything with a Sublime Text `.sublime-syntax` definition.

## At a glance

- **Syntax**: every syntect-supported language, plus drop-in `.sublime-syntax` folders.
- **Themes**: every base16 theme + custom vim highlight imports.
- **Output**: PNG only, with configurable background (solid, horizontal, vertical, or wallpaper image).
- **Pipes well**: `cat foo.rs | sss_code --extension rs > foo.png`.

## When to use it

| Goal | Flag combination |
| --- | --- |
| Render a file | `sss_code path/to/file.rs` |
| Pipe stdin, hint extension | `cat foo.rs \| sss_code -e rs` |
| Pick a theme | `sss_code --theme github` |
| Render only lines 10–40 | `sss_code --lines 10..40 main.rs` |
| Highlight lines 22–25 | `sss_code --highlight-lines 22..25 main.rs` |

Jump to [getting started](/docs/code/getting-started/) for a walkthrough or [flags](/docs/code/flags/) for the full reference.
