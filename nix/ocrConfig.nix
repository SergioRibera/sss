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

  gpu = mkOption {
    type = types.enum [ "auto" "cpu" "cuda" "tensorrt" "coreml" "directml" "openvino" "webgpu" ];
    default = "auto";
    description = ''
      ORT execution provider. `auto` picks the best provider compiled
      into the binary for this host (CoreML on macOS, CUDA when the
      `cli-cuda` package is installed and `/dev/nvidia0` exists,
      otherwise CPU). Setting an explicit value forces that backend; at
      runtime ORT still falls back to CPU when the EP isn't actually
      available in `libonnxruntime`.

      Pair this with the matching package flavour (e.g.
      `programs.sss.package = pkgs.sss.cli-cuda`) — selecting `cuda`
      here without a CUDA-enabled libonnxruntime just falls back to
      CPU.
    '';
  };
}
