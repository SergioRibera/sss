+++
title = "Config reference"
description = "Every `~/.config/sss/config.toml` key, generated from the Nix option modules."
weight = 50
+++

> **This page is auto-generated.** The CI build replaces it with the output of `nix build .#docs`. When you build the site locally without that step, you see this placeholder.

Run the generator manually:

```bash
nix build .#docs -o docs/config.md
cp docs/config.md docs-site/content/docs/config-reference.md
zola serve --root docs-site
```

Source modules: `nix/cliConfig.nix`, `nix/codeConfig.nix`, `nix/sharedConfig.nix`, `nix/captureUiConfig.nix`.
