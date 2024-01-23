{
  crane,
  cranix,
  fenix,
}: {
  config,
  lib,
  pkgs,
  ...
}:
with lib; let
  sss = import ./. {
    inherit crane cranix fenix pkgs lib;
    system = pkgs.system;
  };
  cfg = config.programs.sss;
in {
  options.programs.sss = {
    enable = mkEnableOption "cli to take screenshots";

    code = mkOption {
      type = types.bool;
      default = true;
      description = "Enable sss_code, a sss for code screenshots";
    };
  };

  config = mkIf cfg.enable {
    environment.systemPackages = [sss.packages.default] ++ (lists.optionals cfg.code [sss.packages.code]);
  };
}
