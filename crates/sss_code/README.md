# SSS Code
Terminal tool to take screenshots of your code

![out](https://github.com/SergioRibera/sss/assets/56278796/be74cd48-8f87-4544-98da-c7bc119753ab)

## Usage
> [!NOTE]
> This accepts both files and receiving the code through a stdin 
```sh
Usage: sss_code [OPTIONS] [CONTENT]

Arguments:
  [CONTENT]  Content to take screenshot. It accepts stdin or File

Options:
  -t, --theme <THEME>                                    Theme file to use. May be a path, or an embedded theme. Embedded themes will take precendence. [default:
                                                         base16-ocean.dark]
      --fonts <FONTS>                                    [default: Hack=12.0;] The font used to render, format: Font Name=size;Other Font Name=12.0 [default: Hack=12.0;]
      --vim-theme <VIM_THEME>                            [Not recommended for manual use] Set theme from vim highlights, format: group,bg,fg,style;group,bg,fg,style;
  -l, --list-file-types                                  Lists supported file types
  -L, --list-themes                                      Lists themes
      --extra-syntaxes <EXTRA_SYNTAXES>                  Additional folder to search for .sublime-syntax files in
  -e, --extension <EXTENSION>                            Set the extension of language input
      --lines <LINES>                                    Lines range to take screenshot, format start..end [default: ..]
      --highlight-lines <HIGHLIGHT_LINES>                Lines to highlight over the rest, format start..end [default: ..]
  -n, --line-numbers                                     Show Line numbers
      --tab-width <TAB_WIDTH>                            Tab width [default: 4]
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
  -b, --background <BACKGROUND>                          Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path [default: #323232]
  -r, --radius <RADIUS>                                  [default: 15]
      --padding-x <PADDING_X>                            [default: 80]
      --padding-y <PADDING_Y>                            [default: 100]
      --shadow                                           Enable shadow
      --shadow-image                                     Generate shadow from inner image
      --shadow-color <SHADOW_COLOR>                      Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path [default: #707070]
      --shadow-blur <SHADOW_BLUR>                        [default: 50]
  -c, --just-copy                                        Send the result to your clipboard
  -o, --output <OUTPUT>                                  [values: raw or file path] If it is set then the result will be saved here
  -f, --save-format <SAVE_FORMAT>                        The format in which the image will be saved [default: png]
  -h, --help                                             Print help
  -V, --version                                          Print version
```

## From file
```sh
sss_code --window-controls --window-title example.rs -n --background '#aaaaff' -e rs -f png -o ./out.png ./example.rs
```

## From clipboard (Wayland example)
```sh
wl-paste | sss_code --window-controls --window-title example.rs -n --background '#aaaaff' -e rs -f png -o ./out.png -
```

## To raw clipboard (Wayland example)
```sh
sss_code --window-controls --window-title example.rs -n --background '#aaaaff' -e rs -f png -o raw ./example.rs | wl-copy
```
