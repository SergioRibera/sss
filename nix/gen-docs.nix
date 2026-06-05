{ pkgs ? import <nixpkgs> { }
, lib ? pkgs.lib
}:

let
  cliConfig = import ./cliConfig.nix { inherit lib; };
  codeConfig = import ./codeConfig.nix { inherit lib; };
  sharedConfig = import ./sharedConfig.nix { inherit lib; };
  captureUiConfig = import ./captureUiConfig.nix { inherit lib; };

  module = { lib, ... }: with lib; {
    options.programs.sss = {
      cli = mkOption {
        description = "CLI targeting / backend options.";
        default = { };
        type = types.submodule { options = cliConfig; };
      };

      code = mkOption {
        description = "Settings for `sss_code` (source-code screenshots).";
        default = { };
        type = types.submodule { options = codeConfig; };
      };

      general = mkOption {
        description = "Shared rendering settings used by both `sss` and `sss_code`.";
        default = { };
        type = types.submodule { options = sharedConfig; };
      };

      capture-ui = mkOption {
        description = ''
          Interactive selector / annotation UI configuration: toolbar tools,
          colour palette, default stroke values, snap step, chrome colours.
        '';
        default = { };
        type = types.submodule { options = captureUiConfig; };
      };
    };
  };

  eval = lib.evalModules {
    modules = [ module ];
  };

  optionsDoc = pkgs.nixosOptionsDoc {
    inherit (eval) options;
    documentType = "none";
    transformOptions = opt: opt // {
      declarations = [ ];
    };
  };

  preamble = pkgs.writeText "preamble.md" ''
    # `sss` Configuration Reference

    This document is **auto-generated** from the Nix option modules in
    `nix/` (`cliConfig.nix`, `codeConfig.nix`, `sharedConfig.nix`,
    `captureUiConfig.nix`). To update it, edit the relevant `.nix` file
    and run `nix build .#docs -o docs/config.md` (or `cargo make
    docs-config`).

    Every option corresponds to a key in `~/.config/sss/config.toml` —
    the Home Manager / NixOS modules render that TOML file from your
    `programs.sss` configuration. The TOML section names match the Nix
    attribute path: `programs.sss.general.padding-x` becomes
    `[general]` / `padding-x` in TOML, etc.

    ## `imports` (TOML-only)

    The top-level `imports` array merges other TOML files in before the
    importing file. It is **not** a Nix module option — set it directly
    in your `config.toml`:

    ```toml
    imports = [
      "themes/dark.toml",
      "~/.config/sss/local.toml",
    ]
    ```

    Paths resolve relative to the importing file's directory (or `~/`
    to `$HOME`). Missing files are skipped with a warning — never an
    error, so an optional override file is safe to list. Within a
    file, later entries override earlier ones; the importing file
    overrides all of its imports; CLI flags override everything.
    Cycles are broken with a warning.

  '';

in pkgs.runCommand "sss-config-reference.md"
{
  nativeBuildInputs = [ pkgs.coreutils ];
}
  ''
    cat ${preamble} ${optionsDoc.optionsCommonMark} > "$out"
  ''
