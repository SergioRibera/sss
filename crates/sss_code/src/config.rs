use std::ops::Range;

use clap::Parser;
use clap_stdin::FileOrStdin;
use merge2::{bool::overwrite_false, Merge};
use serde::{Deserialize, Serialize};
use sss_lib::font::parse_font_str;
use sss_lib::font::FontCollection;
use sss_lib::{Background, GenerationSettings, Shadow, ToRgba};

use crate::error::CodeScreenshotError;

const fn default_bool() -> bool {
    false
}

#[inline]
fn swap_option<T>(left: &mut Option<T>, right: &mut Option<T>) {
    if left.is_none() || right.is_some() {
        core::mem::swap(left, right);
    }
}

#[derive(Clone, Debug, Deserialize, Merge, Parser, Serialize)]
#[clap(author, version, about)]
pub struct CodeConfig {
    #[clap(help = "Content to take screenshot. It accepts stdin or File")]
    #[serde(skip)]
    pub content: Option<FileOrStdin<String>>,
    #[clap(
        long,
        short,
        default_value = "base16-ocean.dark",
        help = "Theme file to use. May be a path, or an embedded theme. Embedded themes will take precendence."
    )]
    pub theme: Option<String>,
    #[clap(
        long,
        default_value = "Hack=12.0;",
        help = "The font used to render, format: Font Name=size;Other Font Name=12.0",
        value_parser = parse_font_str
    )]
    pub fonts: Option<FontCollection>,
    #[clap(
        long,
        help = "[Not recommended for manual use] Set theme from vim highlights, format: group,bg,fg,style;group,bg,fg,style;"
    )]
    pub vim_theme: Option<String>,
    // Setting synctect
    #[clap(long, short = 'l', help = "Lists supported file types")]
    #[merge(strategy = overwrite_false)]
    #[serde(skip)]
    pub list_file_types: bool,
    #[clap(long, short = 'L', help = "Lists themes")]
    #[merge(strategy = overwrite_false)]
    #[serde(skip)]
    pub list_themes: bool,
    #[clap(
        long,
        help = "Additional folder to search for .sublime-syntax files in"
    )]
    pub extra_syntaxes: Option<String>,
    #[clap(long, short, help = "Set the extension of language input")]
    #[serde(skip)]
    pub extension: Option<String>,
    // Render options
    #[clap(long, default_value="..", help = "Lines range to take screenshot, format start..end", value_parser=parse_range)]
    #[serde(skip)]
    pub lines: Option<Range<usize>>,
    #[clap(long, default_value="..", help = "Lines to highlight over the rest, format start..end", value_parser=parse_range)]
    #[serde(skip)]
    pub highlight_lines: Option<Range<usize>>,
    #[clap(long, short = 'n', default_value = "false", help = "Show Line numbers")]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub line_numbers: bool,
    #[clap(long, default_value = "4", help = "Tab width")]
    pub tab_width: Option<u8>,
    #[clap(long, help = "Author Name of screenshot")]
    pub author: Option<String>,
    #[clap(long, default_value = "#FFFFFF", help = "Title bar text color")]
    pub author_color: Option<String>,
    #[clap(long, default_value = "Hack", help = "Font to render Author")]
    pub author_font: Option<String>,
    // Window Bar
    #[clap(long, help = "Whether show the window controls")]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub window_controls: bool,
    #[clap(long, help = "Window title")]
    pub window_title: Option<String>,
    #[clap(long, default_value = "#4287f5", help = "Window bar background")]
    pub window_background: Option<String>,
    #[clap(long, default_value = "#FFFFFF", help = "Title bar text color")]
    pub window_title_color: Option<String>,
    #[clap(long, default_value = "120", help = "Width of window controls")]
    pub window_controls_width: Option<u32>,
    #[clap(
        long,
        default_value = "40",
        help = "Height of window title/controls bar"
    )]
    pub window_controls_height: Option<u32>,
    #[clap(long, default_value = "10", help = "Padding of title on window bar")]
    pub titlebar_padding: Option<u32>,
    // Screenshot Section
    #[clap(
        long,
        short,
        default_value = "#323232",
        help = "Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path"
    )]
    pub background: Option<String>,
    #[clap(long, short, default_value = "15")]
    pub radius: Option<u32>,
    // Padding Section
    #[clap(long, default_value = "80")]
    #[merge(strategy = swap_option)]
    pub padding_x: Option<u32>,
    #[clap(long, default_value = "100")]
    #[merge(strategy = swap_option)]
    pub padding_y: Option<u32>,
    // Shadow Section
    #[clap(long, help = "Enable shadow")]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub shadow: bool,
    #[clap(long, help = "Generate shadow from inner image")]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub shadow_image: bool,
    #[clap(
        long,
        default_value = "#707070",
        help = "Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path"
    )]
    #[merge(strategy = swap_option)]
    pub shadow_color: Option<String>,
    #[clap(long, default_value = "50")]
    #[merge(strategy = swap_option)]
    pub shadow_blur: Option<f32>,
    // Saving options
    #[clap(long, short = 'c', help = "Send the result to your clipboard")]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub copy: bool,
    #[clap(
        long,
        short,
        help = "If it is set then the result will be saved here, otherwise it will not be saved."
    )]
    #[serde(skip)]
    pub output: String,
    #[clap(
        long,
        short = 'f',
        default_value = "png",
        help = "The format in which the image will be saved"
    )]
    pub save_format: Option<String>,
}

pub fn get_config() -> CodeConfig {
    let config_path = directories::BaseDirs::new()
        .unwrap()
        .config_dir()
        .join("sss_code");

    let _ = std::fs::create_dir_all(config_path.clone());

    let config_path = config_path.join("config.toml");
    println!("Reading configs from path: {config_path:?}");

    if let Ok(cfg_content) = std::fs::read_to_string(config_path) {
        println!("Merging from config file");
        let mut config: CodeConfig = toml::from_str(&cfg_content).unwrap();
        let mut args = CodeConfig::parse();

        config.merge(&mut args);
        return config;
    }
    CodeConfig::parse()
}

impl From<CodeConfig> for GenerationSettings {
    fn from(val: CodeConfig) -> Self {
        let background = Background::try_from(val.background.unwrap().clone()).unwrap();
        let windows_background =
            Background::try_from(val.window_background.unwrap().clone()).unwrap();
        let shadow_color = Background::try_from(val.shadow_color.unwrap().clone()).unwrap();

        GenerationSettings {
            windows_background,
            background,
            padding: (val.padding_x.unwrap(), val.padding_y.unwrap()),
            round_corner: val.radius,
            shadow: val.shadow.then_some(Shadow {
                shadow_color,
                use_inner_image: val.shadow_image,
                blur_radius: val.shadow_blur.unwrap(),
            }),
            fonts: val.fonts.unwrap_or_default(),
            author: val.author.clone(),
            author_font: val.author_font.clone().unwrap(),
            author_color: val.author_color.unwrap().to_rgba().unwrap(),
            window_controls: val.window_controls,
            windows_title: val.window_title.clone(),
            windows_title_color: val.window_title_color.unwrap().to_rgba().unwrap(),
            window_controls_width: val.window_controls_width.unwrap(),
            window_controls_height: val.window_controls_height.unwrap(),
            titlebar_padding: val.titlebar_padding.unwrap(),
        }
    }
}

fn parse_range(s: &str) -> Result<Range<usize>, CodeScreenshotError> {
    let Some(other) = s.chars().find(|c| !c.is_numeric()) else {
        return Err(CodeScreenshotError::InvalidFormat("range", "start..end"));
    };

    let Some((start_str, end_str)) = s.split_once(&other.to_string()) else {
        return Err(CodeScreenshotError::InvalidFormat("range", "start..end"));
    };

    let (start, end) = (
        start_str
            .replace(other, "")
            .parse::<usize>()
            .map(|s| if s >= 1 { s - 1 } else { s })
            .unwrap_or_default(),
        end_str
            .replace(other, "")
            .parse::<usize>()
            .map(|s| s + 1)
            .unwrap_or(usize::MAX),
    );

    Ok(Range { start, end })
}
