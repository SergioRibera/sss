{ lib, ... }:
with lib; {
  current = mkEnableOption "Capture the screen/window the cursor is on";
  screen = mkEnableOption "Open the monitor selector (or capture the current monitor when combined with `current`)";
  screen-id = mkOption {
    type = types.str;
    default = "";
    description = "ID or Name of screen to capture directly (skips the selector).";
  };
  area = mkOption {
    type = types.str;
    default = "";
    example = "100,100 800x600";
    description = ''
      Capture this rectangle directly, in `x,y WxH` format. Set to
      `interactive` to force the interactive area selector.
    '';
  };
  window = mkOption {
    type = types.str;
    default = "";
    example = "Firefox";
    description = "Pick a window directly by id (numeric) or title substring.";
  };
  show-cursor = mkEnableOption "Composite the cursor into the captured frame";
  interactive = mkEnableOption "Force the interactive selector even when targeting flags carry an explicit value";
  no-toolbar = mkEnableOption "Hide the annotation toolbar in interactive mode (slurp-class picker only)";
  verbose = mkEnableOption "Bump the default log level to `info`";
  capture-backend = mkOption {
    type = types.enum [ "auto" "wayland" "portal" "x11" "windows" "macos" ];
    default = "auto";
    description = "Force a specific capture backend.";
  };
  remember-last-selection = mkEnableOption ''
    Persist the last interactive area selection and pre-seed the selector
    with it next time `--area` is opened without a value. Stored at
    `''${XDG_CONFIG_HOME}/sss/last_selection.toml`.
  '';
}
