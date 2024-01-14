# SSS
Terminal tool to take screenshots of your screen

![out](https://github.com/SergioRibera/sss/assets/56278796/945f224c-96ec-48b6-a738-50ac2c9cfb90)

## Usage

> [!IMPORTANT]
> You need use the slurp format for the area

```sh
Usage: sss [OPTIONS]

Options:
      --current                                          When you take from a screen or window, capture the one on which the mouse is located.
      --show-cursor                                      Capture cursor (Only Wayland)
      --screen                                           Capture a full screen
      --area <AREA>                                      Captures an area of the screen
      --fonts <FONTS>                                    [default: Hack=12.0;] The font used to render, format: Font Name=size;Other Font Name=12.0 [default: Hack=12.0;]
  -b, --background <BACKGROUND>                          Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path [default: #323232]
  -r, --radius <RADIUS>                                  [default: 15]
      --author <AUTHOR>                                  Author Name of screenshot
      --author-color <AUTHOR_COLOR>                      Title bar text color [default: #FFFFFF]
      --author-font <AUTHOR_FONT>                        Font to render Author [default: Hack]
      --window-controls                                  Whether show the window controls
      --window-title <WINDOW_TITLE>                      Window title
      --window-background <WINDOW_BACKGROUND>            Window bar background [default: #4287f5]
      --window-title-color <WINDOW_TITLE_COLOR>          Title bar text color [default: #FFFFFF]
      --window-controls-width <WINDOW_CONTROLS_WIDTH>    Width of window controls [default: 120]
      --window-controls-height <WINDOW_CONTROLS_HEIGHT>  Height of window title/controls bar [default: 40]
      --titlebar-padding <TITLEBAR_PADDING>              Padding of title on window bar [default: 10]
      --padding-x <PADDING_X>                            [default: 80]
      --padding-y <PADDING_Y>                            [default: 100]
      --shadow                                           Enable shadow
      --shadow-image                                     Generate shadow from inner image
      --shadow-color <SHADOW_COLOR>                      Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path [default: #707070]
      --shadow-blur <SHADOW_BLUR>                        [default: 50]
  -c, --just-copy                                        Send the result to your clipboard
  -o, --output <OUTPUT>                                  If it is set then the result will be saved here, otherwise it will not be saved. [default: None]
  -f, --save-format <SAVE_FORMAT>                        The format in which the image will be saved [default: png]
  -h, --help                                             Print help
  -V, --version                                          Print version
```

## Capture Area
```sh
sss --area "$(slurp)" --window-controls --windows-background "#ffffff" --author "SergioRibera" -o out.png
```
