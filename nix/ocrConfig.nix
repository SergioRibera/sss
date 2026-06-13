{ lib, ... }:
with lib; {
  enable = mkOption {
    type = types.nullOr types.bool;
    default = null;
    description = ''
      Run the OCR pipeline after every capture. The Rust default is
      `true` — leave this `null` to inherit that. Set to `false` to
      disable downloading models and the in-overlay text selection.
    '';
  };

  tier = mkOption {
    type = types.enum [ "auto" "light" "standard" "heavy" ];
    default = "auto";
    description = ''
      Model size class. `auto` (the default) inspects core count and
      RAM at startup; `heavy` is required for the formula model.
    '';
  };

  language = mkOption {
    type = types.listOf types.str;
    default = [ "auto" ];
    example = [ "en" "es" "latin" ];
    description = ''
      Recognition languages to pre-download. Accepts ISO 639-1 codes
      (`en`, `es`, `ja`) and PaddleOCR script names (`latin`,
      `cyrillic`, `arabic`). First entry is the active recogniser; the
      rest stay cached for fast switching.
    '';
  };

  formula = mkEnableOption ''
    Pull the formula recognition model alongside the regular text
    models. Only effective at `tier = "heavy"`.
  '';

  models-dir = mkOption {
    type = types.nullOr types.str;
    default = null;
    example = "/var/cache/sss/models";
    description = ''
      Override the on-disk model cache directory. When `null` the OCR
      worker uses `$XDG_DATA_HOME/sss/models` (or the platform
      equivalent). The directory is created on first download.
    '';
  };
}
