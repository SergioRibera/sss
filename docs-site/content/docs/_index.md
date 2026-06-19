+++
title = "Documentation"
description = "Reference for sss_cli, sss_code, sss_capture_ui and the shared config file."
template = "section.html"
sort_by = "weight"
+++

Pick a crate from the cards above, or jump straight to the [config reference](/docs/config-reference/) — every TOML key, auto-generated from the source Nix modules.

The workspace ships three user-facing binaries and one library:

- **`sss`** — interactive screen / window / region capture with an annotation overlay.
- **`sss_code`** — render source files into PNG with syntax highlighting.
- **`sss-select`** + **`sss_capture_ui`** — the selector binary, and the library powering it.

They share a single config file at `~/.config/sss/config.toml`. CLI flags always win over config; config wins over imported TOML files.
