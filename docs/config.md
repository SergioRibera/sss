# `sss` Configuration Reference

This document is **auto-generated** from the Nix option modules in
`nix/` (`cliConfig.nix`, `codeConfig.nix`, `sharedConfig.nix`,
`captureUiConfig.nix`). To update it, edit the relevant `.nix` file
and run `nix build .#docs -o docs/config.md` (or `cargo make
docs-config`).

Every option corresponds to a key in `~/.config/sss/config.toml` —
the Home Manager / NixOS modules render that TOML file from your
`programs.sss` configuration. The TOML section names match the Nix
attribute path: `programs.sss.general.padding-x` becomes
`[general]` / `padding-x` in TOML, etc.

## _module\.args

Additional arguments passed to each module in addition to ones
like ` lib `, ` config `,
and ` pkgs `, ` modulesPath `\.

This option is also available to all submodules\. Submodules do not
inherit args from their parent module, nor do they provide args to
their parent module or sibling submodules\. The sole exception to
this is the argument ` name ` which is provided by
parent modules to a submodule and contains the attribute name
the submodule is bound to, or a unique generated name if it is
not bound to an attribute\.

Some arguments are already passed by default, of which the
following *cannot* be changed with this option:

 - ` lib `: The nixpkgs library\.

 - ` config `: The results of all options after merging the values from all modules together\.

 - ` options `: The options declared in all modules\.

 - ` specialArgs `: The ` specialArgs ` argument passed to ` evalModules `\.

 - All attributes of ` specialArgs `
   
   Whereas option values can generally depend on other option values
   thanks to laziness, this does not apply to ` imports `, which
   must be computed statically before anything else\.
   
   For this reason, callers of the module system can provide ` specialArgs `
   which are available during import resolution\.
   
   For NixOS, ` specialArgs ` includes
   ` modulesPath `, which allows you to import
   extra modules from the nixpkgs package tree without having to
   somehow make the module aware of the location of the
   ` nixpkgs ` or NixOS directories\.
   
   ```
   { modulesPath, ... }: {
     imports = [
       (modulesPath + "/profiles/minimal.nix")
     ];
   }
   ```

For NixOS, the default value for this option includes at least this argument:

 - ` pkgs `: The nixpkgs package set according to
   the ` nixpkgs.pkgs ` option\.



*Type:*
lazy attribute set of raw value



*Default:*

```nix
{ }
```



## programs\.sss\.capture-ui



Interactive selector / annotation UI configuration: toolbar tools,
colour palette, default stroke values, snap step, chrome colours\.



*Type:*
submodule



*Default:*

```nix
{ }
```



## programs\.sss\.capture-ui\.background-dim



Amount to darken pixels outside the active region (0 = no dim, 255 =
black)\. Before any region is drawn, the whole overlay is dimmed by
this value so the desktop reads as inactive\.



*Type:*
integer between 0 and 255 (both inclusive)



*Default:*

```nix
80
```



## programs\.sss\.capture-ui\.chrome



Chrome colours used by the toolbar, popups and radial menu\.



*Type:*
submodule



*Default:*

```nix
{ }
```



## programs\.sss\.capture-ui\.chrome\.accent



Selection / focus accent colour\.



*Type:*
string



*Default:*

```nix
"#5aaaff"
```



## programs\.sss\.capture-ui\.chrome\.button-active-bg



Background of the selected toolbar button\.



*Type:*
string



*Default:*

```nix
"#3c6ec8"
```



## programs\.sss\.capture-ui\.chrome\.button-active-border



Border of the selected toolbar button\.



*Type:*
string



*Default:*

```nix
"#b4dcff"
```



## programs\.sss\.capture-ui\.chrome\.button-bg



Background of an idle toolbar button\.



*Type:*
string



*Default:*

```nix
"#2a2a2e"
```



## programs\.sss\.capture-ui\.chrome\.toolbar-bg



Background colour of the toolbar / popup panels\.



*Type:*
string



*Default:*

```nix
"#161618"
```



## programs\.sss\.capture-ui\.chrome\.toolbar-border



Border colour of the toolbar / popup panels\.



*Type:*
string



*Default:*

```nix
"#505054"
```



## programs\.sss\.capture-ui\.chrome\.toolbar-fg



Text / icon colour on the toolbar\.



*Type:*
string



*Default:*

```nix
"#f0f0f0"
```



## programs\.sss\.capture-ui\.default-blur-radius



Default Gaussian blur radius for the Blur Rectangle tool\.



*Type:*
floating point number



*Default:*

```nix
12.0
```



## programs\.sss\.capture-ui\.default-eraser-radius



Default eraser radius\.



*Type:*
floating point number



*Default:*

```nix
18.0
```



## programs\.sss\.capture-ui\.default-fill



Initial fill colour for closed shapes\. ` null ` disables fill mode at
startup (the user can still toggle fill from the toolbar)\.



*Type:*
null or string



*Default:*

```nix
null
```



*Example:*

```nix
"#80ff00ff"
```



## programs\.sss\.capture-ui\.default-step-radius



Default radius for the numbered-Step tool circles\.



*Type:*
floating point number



*Default:*

```nix
14.0
```



## programs\.sss\.capture-ui\.default-stroke-color



Initial stroke colour for every shape tool\.



*Type:*
string



*Default:*

```nix
"#dc322f"
```



## programs\.sss\.capture-ui\.default-stroke-width



Initial stroke width\.



*Type:*
floating point number



*Default:*

```nix
3.0
```



## programs\.sss\.capture-ui\.default-text-size



Default text size for the Text tool (logical pixels)\.



*Type:*
floating point number



*Default:*

```nix
18.0
```



## programs\.sss\.capture-ui\.initial-tool



Tool the overlay opens with\. Must be present in ` tools `\.



*Type:*
one of “pointer”, “brush”, “line”, “arrow”, “rectangle”, “ellipse”, “polygon”, “blur-rect”, “eraser”, “step”, “text”



*Default:*

```nix
"pointer"
```



## programs\.sss\.capture-ui\.palette



Colour swatches shown in the toolbar and the right-click radial menu\.
Accepts ` #RGB `, ` #RRGGBB ` or ` #RRGGBBAA ` hex strings\.



*Type:*
list of string



*Default:*

```nix
[
  "#dc322f"
  "#ff8c00"
  "#f0c800"
  "#32b450"
  "#3c78e6"
  "#aa5ae6"
  "#000000"
  "#ffffff"
]
```



*Example:*

```nix
[
  "#ff0000"
  "#00ff00"
  "#0000ff"
]
```



## programs\.sss\.capture-ui\.radial-widths



Stroke widths offered in the radial menu’s width row\.



*Type:*
list of floating point number



*Default:*

```nix
[
  1.0
  3.0
  6.0
  12.0
]
```



## programs\.sss\.capture-ui\.region-outline-color



Outline colour for the region rubber-band rectangle\.



*Type:*
string



*Default:*

```nix
"#ffffff"
```



## programs\.sss\.capture-ui\.snap-step



Snap-grid step in pixels (toggled at runtime with ` G `)\.



*Type:*
floating point number



*Default:*

```nix
10.0
```



## programs\.sss\.capture-ui\.tools



Tools shown in the toolbar (and their order)\.



*Type:*
list of (one of “pointer”, “brush”, “line”, “arrow”, “rectangle”, “ellipse”, “polygon”, “blur-rect”, “eraser”, “step”, “text”)



*Default:*

```nix
[
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
]
```



*Example:*

```nix
[
  "pointer"
  "brush"
  "arrow"
  "blur-rect"
]
```



## programs\.sss\.cli



CLI targeting / backend options\.



*Type:*
submodule



*Default:*

```nix
{ }
```



## programs\.sss\.cli\.area



Capture this rectangle directly, in ` x,y WxH ` format\. Set to
` interactive ` to force the interactive area selector\.



*Type:*
string



*Default:*

```nix
""
```



*Example:*

```nix
"100,100 800x600"
```



## programs\.sss\.cli\.capture-backend



Force a specific capture backend\.



*Type:*
one of “auto”, “wayland”, “portal”, “x11”, “windows”, “macos”



*Default:*

```nix
"auto"
```



## programs\.sss\.cli\.current



Whether to enable Capture the screen/window the cursor is on\.



*Type:*
boolean



*Default:*

```nix
false
```



*Example:*

```nix
true
```



## programs\.sss\.cli\.interactive



Whether to enable Force the interactive selector even when targeting flags carry an explicit value\.



*Type:*
boolean



*Default:*

```nix
false
```



*Example:*

```nix
true
```



## programs\.sss\.cli\.no-toolbar



Whether to enable Hide the annotation toolbar in interactive mode (slurp-class picker only)\.



*Type:*
boolean



*Default:*

```nix
false
```



*Example:*

```nix
true
```



## programs\.sss\.cli\.screen



Whether to enable Open the monitor selector (or capture the current monitor when combined with ` current `)\.



*Type:*
boolean



*Default:*

```nix
false
```



*Example:*

```nix
true
```



## programs\.sss\.cli\.screen-id



ID or Name of screen to capture directly (skips the selector)\.



*Type:*
string



*Default:*

```nix
""
```



## programs\.sss\.cli\.show-cursor



Whether to enable Composite the cursor into the captured frame\.



*Type:*
boolean



*Default:*

```nix
false
```



*Example:*

```nix
true
```



## programs\.sss\.cli\.verbose



Whether to enable Bump the default log level to ` info `\.



*Type:*
boolean



*Default:*

```nix
false
```



*Example:*

```nix
true
```



## programs\.sss\.cli\.window



Pick a window directly by id (numeric) or title substring\.



*Type:*
string



*Default:*

```nix
""
```



*Example:*

```nix
"Firefox"
```



## programs\.sss\.code



Settings for ` sss_code ` (source-code screenshots)\.



*Type:*
submodule



*Default:*

```nix
{ }
```



## programs\.sss\.code\.enable



Whether to enable cli to take screenshots of source code\.



*Type:*
boolean



*Default:*

```nix
false
```



*Example:*

```nix
true
```



## programs\.sss\.code\.code-background



Background of code section\. Support: ‘\#RRGGBBAA’ ‘h;\#RRGGBBAA;\#RRGGBBAA’ ‘v;\#RRGGBBAA;\#RRGGBBAA’ or file path



*Type:*
string



*Default:*

```nix
""
```



## programs\.sss\.code\.extension



Force a specific syntax (file extension)\.



*Type:*
string



*Default:*

```nix
""
```



*Example:*

```nix
"rs"
```



## programs\.sss\.code\.extra-syntaxes



Additional folder to search for \.sublime-syntax files in



*Type:*
string



*Default:*

```nix
""
```



*Example:*

```nix
"~/.config/extra-syntaxes"
```



## programs\.sss\.code\.hidden-chars



Hidden characters to display, format ` kind:char `



*Type:*
list of string



*Default:*

```nix
[ ]
```



*Example:*

```nix
[
  "space:·"
  "eol:¶"
  "tab:»"
]
```



## programs\.sss\.code\.indent-chars



Characters used to render each indent level



*Type:*
list of string



*Default:*

```nix
[ ]
```



*Example:*

```nix
[
  "│"
  "┊"
]
```



## programs\.sss\.code\.line-numbers



Whether to enable Show line numbers\.



*Type:*
boolean



*Default:*

```nix
false
```



*Example:*

```nix
true
```



## programs\.sss\.code\.tab-width



Tab width



*Type:*
signed integer



*Default:*

```nix
4
```



*Example:*

```nix
4
```



## programs\.sss\.code\.theme



Theme file to use\. May be a path, or an embedded theme\. Embedded themes take precedence\.



*Type:*
string



*Default:*

```nix
"base16-ocean.dark"
```



*Example:*

```nix
"base16-ocean.dark"
```



## programs\.sss\.code\.vim-theme



\[Not recommended for manual use] Set theme from vim highlights, format: group,bg,fg,style;group,bg,fg,style;



*Type:*
string



*Default:*

```nix
""
```



## programs\.sss\.general



Shared rendering settings used by both ` sss ` and ` sss_code `\.



*Type:*
submodule



*Default:*

```nix
{ }
```



## programs\.sss\.general\.author



Author Name printed as a footer below the screenshot



*Type:*
string



*Default:*

```nix
""
```



## programs\.sss\.general\.author-font



Font used to render the author footer



*Type:*
string



*Default:*

```nix
"Hack"
```



## programs\.sss\.general\.colors



*Type:*
submodule



*Default:*

```nix
{ }
```



## programs\.sss\.general\.colors\.author



Author footer text colour



*Type:*
string



*Default:*

```nix
"#FFFFFF"
```



## programs\.sss\.general\.colors\.background



Background of the generated image\. Support: ‘\#RRGGBBAA’ ‘h;\#RRGGBBAA;\#RRGGBBAA’ ‘v;\#RRGGBBAA;\#RRGGBBAA’ or file path



*Type:*
string



*Default:*

```nix
"#323232"
```



## programs\.sss\.general\.colors\.shadow



Shadow colour\. Same format as ` background `\.



*Type:*
string



*Default:*

```nix
"#707070"
```



## programs\.sss\.general\.colors\.title



Window-title text colour



*Type:*
string



*Default:*

```nix
"#FFFFFF"
```



## programs\.sss\.general\.colors\.window-background



Window-controls bar background\. Same format as ` background `\.



*Type:*
string



*Default:*

```nix
""
```



## programs\.sss\.general\.copy



Whether to enable Copy screenshot to clipboard\.



*Type:*
boolean



*Default:*

```nix
false
```



*Example:*

```nix
true
```



## programs\.sss\.general\.fonts



The font used to render, format: ` Font Name=size;Other Font Name=12.0 `



*Type:*
string



*Default:*

```nix
"Hack=12.0"
```



*Example:*

```nix
"Hack=12.0;Noto Font Emoji=12.0;"
```



## programs\.sss\.general\.notify



Whether to enable Show a desktop notification when the screenshot is saved\.



*Type:*
boolean



*Default:*

```nix
false
```



*Example:*

```nix
true
```



## programs\.sss\.general\.output



Save destination\. Empty means “let the interactive Save button choose”,
` raw ` writes PNG to stdout, anything else is treated as a file path\.



*Type:*
string



*Default:*

```nix
""
```



*Example:*

```nix
"~/Pictures/sss.png"
```



## programs\.sss\.general\.padding-x



Horizontal padding around the screenshot



*Type:*
signed integer



*Default:*

```nix
80
```



## programs\.sss\.general\.padding-y



Vertical padding around the screenshot



*Type:*
signed integer



*Default:*

```nix
100
```



## programs\.sss\.general\.radius



Radius for the rounded screenshot corners



*Type:*
signed integer



*Default:*

```nix
15
```



## programs\.sss\.general\.save-format



Image format used when saving to disk\.



*Type:*
one of “png”, “jpeg”, “webp”



*Default:*

```nix
"png"
```



## programs\.sss\.general\.shadow



Whether to enable Enable shadows\.



*Type:*
boolean



*Default:*

```nix
false
```



*Example:*

```nix
true
```



## programs\.sss\.general\.shadow-blur



Blur radius of the shadow



*Type:*
floating point number



*Default:*

```nix
50.0
```



## programs\.sss\.general\.shadow-image



Whether to enable Generate shadow from the captured image instead of a flat colour\.



*Type:*
boolean



*Default:*

```nix
false
```



*Example:*

```nix
true
```



## programs\.sss\.general\.window-controls



*Type:*
submodule



*Default:*

```nix
{ }
```



## programs\.sss\.general\.window-controls\.enable



Whether to enable Enable the macOS-style window controls bar\.



*Type:*
boolean



*Default:*

```nix
false
```



*Example:*

```nix
true
```



## programs\.sss\.general\.window-controls\.height



Height of the window title / controls bar (px)



*Type:*
signed integer



*Default:*

```nix
40
```



## programs\.sss\.general\.window-controls\.title



Window title shown in the controls bar



*Type:*
string



*Default:*

```nix
""
```



## programs\.sss\.general\.window-controls\.titlebar-padding



Padding of the title inside the controls bar (px)



*Type:*
signed integer



*Default:*

```nix
10
```



## programs\.sss\.general\.window-controls\.width



Width of the window controls (px)



*Type:*
signed integer



*Default:*

```nix
120
```


