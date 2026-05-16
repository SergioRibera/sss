{ crane
, fenix
,
}: { config
   , lib
   , pkgs
   , ...
   }:
with lib; let
  inherit (attrsets) filterAttrs;
  sss = import ./. {
    inherit crane fenix pkgs lib;
    system = pkgs.system;
  };
  cfgSSS = config.programs.sss;
  tomlFormat = pkgs.formats.toml { };
  configDir =
    if pkgs.stdenv.isDarwin
    then "Library/Application Support"
    else config.xdg.configHome;
  cliConfig = import ./cliConfig.nix { inherit lib; };
  codeConfig = import ./codeConfig.nix { inherit lib; };
  sharedConfig = import ./sharedConfig.nix { inherit lib; };
  captureUiConfig = import ./captureUiConfig.nix { inherit lib; };
  sssPackage = lists.optional cfgSSS.enable sss.packages.default;
  codePackage = lists.optional cfgSSS.code.enable sss.packages.code;
  filterConfig = cfg: filterAttrs (n: v: ((builtins.typeOf v) != "null") && n != "enable") cfg;
in
{
  options.programs = {
    sss = {
      enable = mkEnableOption "cli to take screenshots";

      cli = mkOption {
        description = "CLI-specific settings (targeting, backend selection, …).";
        default = { };
        type = types.submodule { options = cliConfig; };
      };

      code = mkOption {
        description = "Settings for `sss_code` (source-code screenshots).";
        default = { };
        type = types.submodule { options = codeConfig; };
      };

      general = mkOption {
        description = "Shared rendering settings (background, padding, shadow, fonts, …).";
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

  config = mkIf (cfgSSS.enable || cfgSSS.code.enable) {
    home.packages = sssPackage ++ codePackage;

    home.file."${configDir}/sss/config.toml" = mkIf (cfgSSS.enable || cfgSSS.code.enable) {
      source =
        tomlFormat.generate "config.toml" (filterConfig cfgSSS);
    };
  };
}
