{ lib, ... }:
with lib; {
  # Toolbar tool list, in visual order.
  tools = mkOption {
    type = types.listOf (types.enum [
      "pointer"
      "brush"
      "line"
      "arrow"
      "rectangle"
      "ellipse"
      "polygon"
      "blur-rect"
      "eraser"
      "step"
      "text"
    ]);
    default = [
      "pointer"
      "brush"
      "line"
      "arrow"
      "rectangle"
      "ellipse"
      "polygon"
      "blur-rect"
      "eraser"
      "step"
    ];
    example = [ "pointer" "brush" "arrow" "blur-rect" ];
    description = "Tools shown in the toolbar (and their order).";
  };

  initial-tool = mkOption {
    type = types.enum [
      "pointer"
      "brush"
      "line"
      "arrow"
      "rectangle"
      "ellipse"
      "polygon"
      "blur-rect"
      "eraser"
      "step"
      "text"
    ];
    default = "pointer";
    description = "Tool the overlay opens with. Must be present in `tools`.";
  };

  palette = mkOption {
    type = types.listOf types.str;
    default = [
      "#dc322f"
      "#ff8c00"
      "#f0c800"
      "#32b450"
      "#3c78e6"
      "#aa5ae6"
      "#000000"
      "#ffffff"
    ];
    example = [ "#ff0000" "#00ff00" "#0000ff" ];
    description = ''
      Colour swatches shown in the toolbar and the right-click radial menu.
      Accepts `#RGB`, `#RRGGBB` or `#RRGGBBAA` hex strings.
    '';
  };

  radial-widths = mkOption {
    type = types.listOf types.float;
    default = [ 1.0 3.0 6.0 12.0 ];
    description = "Stroke widths offered in the radial menu's width row.";
  };

  default-stroke-color = mkOption {
    type = types.str;
    default = "#dc322f";
    description = "Initial stroke colour for every shape tool.";
  };

  default-stroke-width = mkOption {
    type = types.float;
    default = 3.0;
    description = "Initial stroke width.";
  };

  default-fill = mkOption {
    type = types.nullOr types.str;
    default = null;
    example = "#80ff00ff";
    description = ''
      Initial fill colour for closed shapes. `null` disables fill mode at
      startup (the user can still toggle fill from the toolbar).
    '';
  };

  default-blur-radius = mkOption {
    type = types.float;
    default = 12.0;
    description = "Default Gaussian blur radius for the Blur Rectangle tool.";
  };

  default-eraser-radius = mkOption {
    type = types.float;
    default = 18.0;
    description = "Default eraser radius.";
  };

  default-step-radius = mkOption {
    type = types.float;
    default = 14.0;
    description = "Default radius for the numbered-Step tool circles.";
  };

  default-text-size = mkOption {
    type = types.float;
    default = 18.0;
    description = "Default text size for the Text tool (logical pixels).";
  };

  snap-step = mkOption {
    type = types.float;
    default = 10.0;
    description = "Snap-grid step in pixels (toggled at runtime with `G`).";
  };

  region-outline-color = mkOption {
    type = types.str;
    default = "#ffffff";
    description = "Outline colour for the region rubber-band rectangle.";
  };

  background-dim = mkOption {
    type = types.ints.between 0 255;
    default = 80;
    description = ''
      Amount to darken pixels outside the active region (0 = no dim, 255 =
      black). Before any region is drawn, the whole overlay is dimmed by
      this value so the desktop reads as inactive.
    '';
  };

  chrome = mkOption {
    default = { };
    type = types.submodule {
      config = { };
      options = {
        toolbar-bg = mkOption {
          type = types.str;
          default = "#161618";
          description = "Background colour of the toolbar / popup panels.";
        };
        toolbar-fg = mkOption {
          type = types.str;
          default = "#f0f0f0";
          description = "Text / icon colour on the toolbar.";
        };
        toolbar-border = mkOption {
          type = types.str;
          default = "#505054";
          description = "Border colour of the toolbar / popup panels.";
        };
        button-bg = mkOption {
          type = types.str;
          default = "#2a2a2e";
          description = "Background of an idle toolbar button.";
        };
        button-active-bg = mkOption {
          type = types.str;
          default = "#3c6ec8";
          description = "Background of the selected toolbar button.";
        };
        button-active-border = mkOption {
          type = types.str;
          default = "#b4dcff";
          description = "Border of the selected toolbar button.";
        };
        accent = mkOption {
          type = types.str;
          default = "#5aaaff";
          description = "Selection / focus accent colour.";
        };
      };
    };
    description = "Chrome colours used by the toolbar, popups and radial menu.";
  };
}
