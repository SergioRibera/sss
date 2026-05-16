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

  '';

in pkgs.runCommand "sss-config-reference.md"
{
  nativeBuildInputs = [ pkgs.coreutils ];
}
  ''
    cat ${preamble} ${optionsDoc.optionsCommonMark} > "$out"
  ''
