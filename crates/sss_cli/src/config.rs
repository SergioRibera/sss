use std::path::PathBuf;

use clap::Parser;
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
    #[clap(
        long,
        short,
        default_value = "#323232",
        help = "Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path"
    )]
    pub background: String,
    #[clap(long, short, default_value = "15")]
    pub radius: u32,
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
        help = "Support: '#RRGGBBAA' '#RRGGBBAA;#RRGGBBAA' or file path"
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
        GenerationSettings {
            background: background.clone(),
            padding: (val.padding_x, val.padding_y),
            round_corner: Some(val.radius),
            shadow: val.shadow.then_some(Shadow {
                background,
                use_inner_image: val.shadow_image,
                shadow_color: val.shadow_color.to_rgba().unwrap(),
                blur_radius: val.shadow_blur,
            }),
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

fn str_to_area(s: &str) -> Result<(i32, i32, u32, u32), String> {
    let err = "The format of area is wrong (x,y WxH)".to_string();
    let (pos, size) = s.split_once(" ").ok_or(err.clone())?;
    let (x, y) = pos.split_once(",").ok_or(err.clone()).map(|(x, y)| {
        (
            x.parse::<i32>().map_err(|e| e.to_string()),
            y.parse::<i32>().map_err(|e| e.to_string()),
        )
    })?;
    let (w, h) = size.split_once("x").ok_or(err.clone()).map(|(w, h)| {
        (
            w.parse::<u32>().map_err(|e| e.to_string()),
            h.parse::<u32>().map_err(|e| e.to_string()),
        )
    })?;

    Ok((x?, y?, w?, h?))
}
