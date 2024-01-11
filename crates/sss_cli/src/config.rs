use std::path::PathBuf;

use clap::Parser;
use sss_lib::error::FontError;
use sss_lib::font::FontCollection;
use sss_lib::image::{
    error::{ImageFormatHint, UnsupportedError, UnsupportedErrorKind},
    ImageError, ImageFormat,
};
use sss_lib::{Background, GenerationSettings, Shadow, ToRgba};

#[derive(Clone, Debug, Parser)]
#[clap(version, author)]
pub struct CliConfig {
    #[clap(
        long,
        default_value = "false",
        help = "When you take from a screen or window, capture the one on which the mouse is located."
    )]
    pub current: bool,
    #[clap(long, default_value = "false", help = "Capture cursor (Only Wayland)")]
    pub show_cursor: bool,
    #[clap(long, default_value = "false", help = "Capture a full screen")]
    pub screen: bool,
    #[clap(long, help = "ID of screen to capture")]
    pub screen_id: Option<u32>,
    #[clap(long, help = "Captures an area of the screen", value_parser = str_to_area)]
    pub area: Option<(i32, i32, u32, u32)>,
    // Screenshot Section
    #[clap(long, default_value = "Hack=12.0;", help = "[default: Hack=12.0;] The font used to render, format: Font Name=size;Other Font Name=12.0", value_parser = parse_font_str)]
    pub font: FontCollection,
    #[clap(
        long,
        short,
        default_value = "#323232",
        help = "Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path"
    )]
    pub background: String,
    #[clap(long, short, default_value = "15")]
    pub radius: u32,
    #[clap(long, help = "Author Name of screenshot")]
    pub author: Option<String>,
    #[clap(long, default_value = "#FFFFFF", help = "Title bar text color")]
    pub author_color: String,
    #[clap(long, default_value = "Hack", help = "Font to render Author")]
    pub author_font: String,
    // Window Bar
    #[clap(long, help = "Whether show the window controls")]
    pub window_controls: bool,
    #[clap(long, help = "Window title")]
    pub window_title: Option<String>,
    #[clap(long, default_value = "#4287f5", help = "Window bar background")]
    pub windows_background: String,
    #[clap(long, default_value = "#FFFFFF", help = "Title bar text color")]
    pub windows_title_color: String,
    #[clap(long, default_value = "120", help = "Width of window controls")]
    pub window_controls_width: u32,
    #[clap(
        long,
        default_value = "40",
        help = "Height of window title/controls bar"
    )]
    pub window_controls_height: u32,
    #[clap(long, default_value = "10", help = "Padding of title on window bar")]
    pub titlebar_padding: u32,
    // Padding Section
    #[clap(long, default_value = "80")]
    pub padding_x: u32,
    #[clap(long, default_value = "100")]
    pub padding_y: u32,
    // Shadow Section
    #[clap(long, help = "Enable shadow")]
    pub shadow: bool,
    #[clap(
        long,
        default_value = "false",
        help = "Generate shadow from inner image"
    )]
    pub shadow_image: bool,
    #[clap(
        long,
        default_value = "#707070",
        help = "Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path"
    )]
    pub shadow_color: String,
    #[clap(long, default_value = "50")]
    pub shadow_blur: f32,
    // Saving options
    #[clap(long, short = 'c', help = "Send the result to your clipboard")]
    pub just_copy: bool,
    #[clap(
        long,
        default_value = "None",
        help = "If it is set then the result will be saved here, otherwise it will not be saved."
    )]
    pub save_path: Option<PathBuf>,
    #[clap(
        long,
        short = 'f',
        default_value = "png",
        help = "The format in which the image will be saved",
        value_parser = str_to_format
    )]
    pub save_format: ImageFormat,
}

pub fn get_config() -> CliConfig {
    CliConfig::parse()
}

impl From<CliConfig> for GenerationSettings {
    fn from(val: CliConfig) -> Self {
        let background = Background::try_from(val.background.clone()).unwrap();
        let windows_background = Background::try_from(val.windows_background.clone()).unwrap();
        let shadow_color = Background::try_from(val.shadow_color.clone()).unwrap();

        GenerationSettings {
            windows_background,
            background,
            padding: (val.padding_x, val.padding_y),
            round_corner: Some(val.radius),
            shadow: val.shadow.then_some(Shadow {
                shadow_color,
                use_inner_image: val.shadow_image,
                blur_radius: val.shadow_blur,
            }),
            fonts: val.font,
            author: val.author.clone(),
            author_font: val.author_font.clone(),
            author_color: val.author_color.to_rgba().unwrap(),
            window_controls: val.window_controls,
            windows_title: val.window_title.clone(),
            windows_title_color: val.windows_title_color.to_rgba().unwrap(),
            window_controls_width: val.window_controls_width,
            window_controls_height: val.window_controls_height,
            titlebar_padding: val.titlebar_padding,
        }
    }
}

fn str_to_format(s: &str) -> Result<ImageFormat, ImageError> {
    ImageFormat::from_extension(s).ok_or(ImageError::Unsupported(
        UnsupportedError::from_format_and_kind(
            ImageFormatHint::Name(s.to_string()),
            UnsupportedErrorKind::Format(ImageFormatHint::Name(s.to_string())),
        ),
    ))
}

fn parse_font_str(s: &str) -> Result<FontCollection, FontError> {
    let fonts = s
        .split(';')
        .filter(|&f| !f.is_empty())
        .map(|f| {
            let (name, size) = f.split_once('=').unwrap();
            (name.to_owned(), size.parse::<f32>().unwrap_or(26.))
        })
        .collect::<Vec<(String, f32)>>();

    FontCollection::new(&fonts)
}

fn str_to_area(s: &str) -> Result<(i32, i32, u32, u32), String> {
    let err = "The format of area is wrong (x,y WxH)".to_string();
    let (pos, size) = s.split_once(' ').ok_or(err.clone())?;
    let (x, y) = pos.split_once(',').ok_or(err.clone()).map(|(x, y)| {
        (
            x.parse::<i32>().map_err(|e| e.to_string()),
            y.parse::<i32>().map_err(|e| e.to_string()),
        )
    })?;
    let (w, h) = size.split_once('x').ok_or(err.clone()).map(|(w, h)| {
        (
            w.parse::<u32>().map_err(|e| e.to_string()),
            h.parse::<u32>().map_err(|e| e.to_string()),
        )
    })?;

    Ok((x?, y?, w?, h?))
}
