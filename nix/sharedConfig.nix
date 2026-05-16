{ lib, ... }:
with lib; {
  copy = mkEnableOption "Copy screenshot to clipboard";
  shadow = mkEnableOption "Enable shadows";
  shadow-image = mkEnableOption "Generate shadow from the captured image instead of a flat colour";
  notify = mkEnableOption "Show a desktop notification when the screenshot is saved";

  fonts = mkOption {
    type = types.str;
    default = "Hack=12.0";
    example = "Hack=12.0;Noto Font Emoji=12.0;";
    description = "The font used to render, format: `Font Name=size;Other Font Name=12.0`";
  };

  radius = mkOption {
    type = types.int;
    default = 15;
    description = "Radius for the rounded screenshot corners";
  };

  author = mkOption {
    type = types.str;
    default = "";
    description = "Author Name printed as a footer below the screenshot";
  };

  author-font = mkOption {
    type = types.str;
    default = "Hack";
    description = "Font used to render the author footer";
  };

  padding-x = mkOption {
    type = types.int;
    default = 80;
    description = "Horizontal padding around the screenshot";
  };

  padding-y = mkOption {
    type = types.int;
    default = 100;
    description = "Vertical padding around the screenshot";
  };

  shadow-blur = mkOption {
    type = types.float;
    default = 50.0;
    description = "Blur radius of the shadow";
  };

  output = mkOption {
    type = types.str;
    default = "";
    example = "~/Pictures/sss.png";
    description = ''
      Save destination. Empty means "let the interactive Save button choose",
      `raw` writes PNG to stdout, anything else is treated as a file path.
    '';
  };

  save-format = mkOption {
    type = types.enum [ "png" "jpeg" "webp" ];
    default = "png";
    description = "Image format used when saving to disk.";
  };

  colors = mkOption {
    default = { };
    description = "";
    type = types.submodule {
      config = { };
      options = {
        background = mkOption {
          type = types.str;
          default = "#323232";
          description = "Background of the generated image. Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path";
        };
        author = mkOption {
          type = types.str;
          default = "#FFFFFF";
          description = "Author footer text colour";
        };
        window-background = mkOption {
          type = types.str;
          default = "";
          description = "Window-controls bar background. Same format as `background`.";
        };
        shadow = mkOption {
          type = types.str;
          default = "#707070";
          description = "Shadow colour. Same format as `background`.";
        };
        title = mkOption {
          type = types.str;
          default = "#FFFFFF";
          description = "Window-title text colour";
        };
      };
    };
  };

  window-controls = mkOption {
    default = { };
    description = "";
    type = types.submodule {
      config = { };
      options = {
        enable = mkEnableOption "Enable the macOS-style window controls bar";
        title = mkOption {
          type = types.str;
          default = "";
          description = "Window title shown in the controls bar";
        };
        width = mkOption {
          type = types.int;
          default = 120;
          description = "Width of the window controls (px)";
        };
        height = mkOption {
          type = types.int;
          default = 40;
          description = "Height of the window title / controls bar (px)";
        };
        titlebar-padding = mkOption {
          type = types.int;
          default = 10;
          description = "Padding of the title inside the controls bar (px)";
        };
      };
    };
  };
}
