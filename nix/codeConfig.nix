{ lib, ... }:
with lib; {
  enable = mkEnableOption "cli to take screenshots of source code";
  line-numbers = mkEnableOption "Show line numbers";
  code-background = mkOption {
    type = types.str;
    default = "";
    description = "Background of code section. Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path";
  };
  theme = mkOption {
    type = types.str;
    default = "base16-ocean.dark";
    example = "base16-ocean.dark";
    description = "Theme file to use. May be a path, or an embedded theme. Embedded themes take precedence.";
  };
  vim-theme = mkOption {
    type = types.str;
    default = "";
    description = "[Not recommended for manual use] Set theme from vim highlights, format: group,bg,fg,style;group,bg,fg,style;";
  };
  extra-syntaxes = mkOption {
    type = types.str;
    default = "";
    example = "~/.config/extra-syntaxes";
    description = "Additional folder to search for .sublime-syntax files in";
  };
  extension = mkOption {
    type = types.str;
    default = "";
    example = "rs";
    description = "Force a specific syntax (file extension).";
  };
  tab-width = mkOption {
    type = types.int;
    default = 4;
    example = 4;
    description = "Tab width";
  };
  indent-chars = mkOption {
    type = types.listOf types.str;
    default = [ ];
    example = [ "│" "┊" ];
    description = "Characters used to render each indent level";
  };
  hidden-chars = mkOption {
    type = types.listOf types.str;
    default = [ ];
    example = [ "space:·" "eol:¶" "tab:»" ];
    description = "Hidden characters to display, format `kind:char`";
  };
}
