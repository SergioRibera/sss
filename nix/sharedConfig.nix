{ lib, ... }:
with lib; {
  copy = mkEnableOption "Copy screenshot to clipboard";
  shadow = mkEnableOption "Enable shadows";
  shadow-image = mkEnableOption "Enable shadows from captured image";
  fonts = mkOption {
    type = types.str;
    default = "Hack=12.0";
    example = "Hack=12.0;Noto Font Emoji=12.0;";
    description = "The font used to render, format: Font Name=size;Other Font Name=12.0";
  };
  radius = mkOption {
    type = types.int;
    default = 15;
    description = "Radius for the screenshot corners";
  };
  author = mkOption {
    type = types.str;
    default = "";
    description = "Author Name of screenshot";
  };
  window-title = mkOption {
    type = types.str;
    default = "";
    description = "Window title";
  };
  padding-x = mkOption {
    type = types.int;
    default = 80;
    description = "Padding X of inner screenshot";
  };
  padding-y = mkOption {
    type = types.int;
    default = 100;
    description = "Padding Y of inner screenshot";
  };
  shadow-blur = mkOption {
    type = types.float;
    default = 50.0;
    description = "Blur of shadow";
  };
  save-format = mkOption {
    type = types.enum [ "png" "jpeg" "webp" ];
    default = "png";
    description = "The format in which the image will be saved";
  };

  colors = mkOption {
    default = { };
    type = types.submodule {
      config = { };
      options = {
        background = mkOption {
          type = types.str;
          default = "#323232";
          description = "Background of image generated. Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path";
        };
        author = mkOption {
          type = types.str;
          default = "#FFFFFF";
          description = "Title bar text color";
        };
        window-background = mkOption {
          type = types.str;
          default = "#4287f5";
          description = "Window title bar background";
        };
        shadow = mkOption {
          type = types.str;
          default = "#707070";
          description = "Shadow of screenshot. Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path";
        };
        title = mkOption {
          type = types.str;
          default = "#FFFFFF";
          description = "Title bar text color";
        };
      };
    };
  };

  window-controls = mkOption {
    default = { };
    type = types.submodule {
      config = { };
      options = {
        enable = mkEnableOption "cli to take screenshots";
        width = mkOption {
          type = types.int;
          default = 120;
          description = "Width of window controls";
        };
        height = mkOption {
          type = types.int;
          default = 40;
          description = "Height of window title/controls bar";
        };
        titlebar-padding = mkOption {
          type = types.int;
          default = 10;
          description = "Padding of title on window bar";
        };
      };
    };
  };
}
