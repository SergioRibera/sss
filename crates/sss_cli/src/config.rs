use std::path::PathBuf;

use clap::Parser;
use sss_lib::image::{
    error::{ImageFormatHint, UnsupportedError, UnsupportedErrorKind},
    ImageError, ImageFormat,
};
use sss_lib::{Background, GenerationSettings, Shadow, ToRgba};
use sss_select::SelectConfig;

#[derive(Clone, Debug, Parser)]
#[clap(version, author)]
pub struct CliConfig {
    #[clap(
        long,
        default_value = "false",
        help = "When you take from a screen or window, capture the one on which the mouse is located."
    )]
    pub current: bool,
    #[clap(long, default_value = "false", help = "Capture a full screen")]
    pub screen: bool,
    #[clap(long, default_value = "false", help = "Capture a application window")]
    pub window: bool,
    #[clap(long, default_value = "false", help = "Captures an area of the screen")]
    pub area: bool,
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

impl Into<GenerationSettings> for CliConfig {
    fn into(self) -> GenerationSettings {
        let background = Background::try_from(self.background.clone()).unwrap();
        GenerationSettings {
            background: background.clone(),
            padding: (self.padding_x, self.padding_y),
            round_corner: Some(self.radius),
            shadow: Some(Shadow {
                background,
                use_inner_image: self.shadow_image,
                shadow_color: self.shadow_color.to_rgba().unwrap(),
                blur_radius: self.shadow_blur,
            }),
        }
    }
}

impl Into<SelectConfig> for CliConfig {
    fn into(self) -> SelectConfig {
        SelectConfig {
            current: self.current,
            screen: self.screen,
            window: self.window,
            area: self.area,
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
