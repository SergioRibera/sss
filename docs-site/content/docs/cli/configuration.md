+++
title = "Configuration"
description = "TOML layout, import semantics, and override precedence."
weight = 30
+++

## File location

`sss` reads `$XDG_CONFIG_HOME/sss/config.toml` (defaults to `~/.config/sss/config.toml` on Linux/macOS). Override with `--config <path>`.

## Sections

```toml
imports = ["themes/dark.toml", "~/.config/sss/local.toml"]

[general]
# Shared rendering: padding, shadow, fonts, border.

[capture-ui]
# Selector + annotation overlay tooling.

[ocr]
# OCR engine: enable/disable, language, GPU mode.
```

Every key is documented on the [config reference](/docs/config-reference/) page, generated from `nix/sharedConfig.nix`, `nix/captureUiConfig.nix`, `nix/cliConfig.nix`, `nix/codeConfig.nix`.

## Imports

The top-level `imports` array merges other TOML files **before** the importing file. Paths resolve relative to the importing file's directory; `~/` expands to `$HOME`. Missing files are skipped with a warning.

Within a file, later entries override earlier ones. The importing file overrides all of its imports. CLI flags override everything.

```toml
imports = [
  "common.toml",         # base values
  "themes/dark.toml",    # overrides common
  "~/.local-overrides",  # overrides themes
]

[general]
padding-x = 24    # overrides any value imports brought in
```

Cycles are broken with a warning — never an error.

## Precedence (highest to lowest)

1. CLI flags (`--padding-x 24`)
2. Loaded config file values
3. Imported TOML files (last import wins among them)
4. Built-in defaults

## Home Manager / NixOS

If you use Home Manager, set `programs.sss.*` instead — the module renders an equivalent `config.toml` for you. Same TOML keys; the Nix attribute path mirrors the section layout.

```nix
{
  programs.sss = {
    enable = true;
    general.padding-x = 24;
    capture-ui.toolbar = true;
    ocr.gpu = "auto";
  };
}
```
