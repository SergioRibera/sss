{ crane
, fenix
,
}: { config
   , lib
   , options
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
  optSSS = options.programs.sss;
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
  # Emit only the keys the user actually set. We can't use the evaluated
  # `config` view because defaults already filled it in — the Rust side
  # supplies its own defaults via `#[serde(default)]`, so anything the
  # user did not touch must be omitted (otherwise empty strings, falses,
  # and nulls leak in and break parsing). `options.<path>.definitions`
  # returns the raw user-supplied attrset for each section, which we
  # merge and then strip null leaves from.
  mergeDefs = opt: foldl' recursiveUpdate { } (opt.definitions or [ ]);
  # Drop null leaves AND empty subtables — `pkgs.formats.toml` would
  # otherwise emit bare `[section]` headers for untouched sections.
  cleanse = v:
    if builtins.isAttrs v && !(lib.isDerivation v)
    then
      filterAttrs
        (_: x: x != null && !(builtins.isAttrs x && x == { }))
        (builtins.mapAttrs (_: cleanse) v)
    else v;
  userTomlConfig = cleanse (filterAttrs (n: _: n != "enable") {
    cli = mergeDefs optSSS.cli;
    code = mergeDefs optSSS.code;
    general = mergeDefs optSSS.general;
    capture-ui = mergeDefs optSSS.capture-ui;
  });
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
      source = tomlFormat.generate "config.toml" userTomlConfig;
    };
  };
}
