use clap::Parser;
use image::error::{ImageFormatHint, UnsupportedError, UnsupportedErrorKind};
use image::{ImageError, ImageFormat, Rgba};
use merge2::bool::overwrite_false;
use merge2::Merge;
use serde::{Deserialize, Serialize};

use crate::font::{parse_font_str, FontCollection};
use crate::{Background, Colors, GenerationSettings, Shadow, ToRgba, WindowControls};

pub fn str_to_format(s: impl ToString) -> Result<ImageFormat, ImageError> {
    ImageFormat::from_extension(s.to_string()).ok_or(ImageError::Unsupported(
        UnsupportedError::from_format_and_kind(
            ImageFormatHint::Name(s.to_string()),
            UnsupportedErrorKind::Format(ImageFormatHint::Name(s.to_string())),
        ),
    ))
}

pub const fn default_bool() -> bool {
    false
}

#[inline]
pub fn swap_option<T>(left: &mut Option<T>, right: &mut Option<T>) {
    if left.is_none() || right.is_some() {
        core::mem::swap(left, right);
    }
}

#[derive(Clone, Debug, Parser, Merge, Serialize, Deserialize)]
pub struct GenerationSettingsArgs {
    // Screenshot Section
    #[clap(
        long,
        help = "[default: Hack=12.0;] The font used to render, format: Font Name=size;Other Font Name=12.0",
        value_parser = parse_font_str
    )]
    #[merge(strategy = swap_option)]
    pub fonts: Option<FontCollection>,
    #[clap(long, short, help = "[default: 15] ")]
    #[merge(strategy = swap_option)]
    pub radius: Option<u32>,
    #[clap(long, help = "Author Name of screenshot")]
    #[merge(strategy = swap_option)]
    pub author: Option<String>,
    #[clap(long, help = "[default: Hack] Font to render Author")]
    #[merge(strategy = swap_option)]
    pub author_font: Option<String>,
    // Padding Section
    #[clap(long, help = "[default: 80]")]
    #[merge(strategy = swap_option)]
    pub padding_x: Option<u32>,
    #[clap(long, help = "[default: 100]")]
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
    #[clap(long, help = "[default: 50] Shadow blur")]
    #[merge(strategy = swap_option)]
    pub shadow_blur: Option<f32>,
    // Saving options
    #[clap(long, short, help = "Send the result to your clipboard")]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub copy: bool,
    #[clap(
        long,
        short,
        help = "[values: raw or file path] If it is set then the result will be saved here"
    )]
    #[serde(skip)]
    pub output: String,
    #[clap(
        long,
        short = 'f',
        help = "[default: png] The format in which the image will be saved"
    )]
    #[merge(strategy = swap_option)]
    pub save_format: Option<String>,
    #[clap(flatten)]
    pub colors: ColorsArgs,
    #[clap(flatten)]
    #[serde(rename = "window-controls")]
    pub window_controls: WindowControlsArgs,
}

#[derive(Clone, Debug, Parser, Merge, Serialize, Deserialize)]
pub struct ColorsArgs {
    #[clap(
        long,
        short,
        help = "[default: #323232] Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path"
    )]
    #[merge(strategy = swap_option)]
    pub background: Option<String>,
    #[clap(long, help = "[default: #FFFFFF] Title bar text color")]
    #[merge(strategy = swap_option)]
    #[serde(rename = "author")]
    pub author_color: Option<String>,
    #[clap(long, help = "[default: #4287f5] Window bar background")]
    #[merge(strategy = swap_option)]
    pub window_background: Option<String>,
    #[clap(long, help = "[default: #FFFFFF] Title bar text color")]
    #[merge(strategy = swap_option)]
    #[serde(rename = "title")]
    pub window_title_color: Option<String>,
    #[clap(
        long,
        help = "[default: #707070] Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path"
    )]
    #[merge(strategy = swap_option)]
    #[serde(rename = "shadow")]
    pub shadow_color: Option<String>,
}

#[derive(Clone, Debug, Parser, Merge, Serialize, Deserialize)]
pub struct WindowControlsArgs {
    // Window Bar
    #[clap(long = "window-controls", help = "Whether show the window controls")]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub enable: bool,
    #[clap(long, help = "Window title")]
    #[merge(strategy = swap_option)]
    #[serde(rename = "title")]
    pub window_title: Option<String>,
    #[clap(long, help = "[default 120] Width of window controls")]
    #[merge(strategy = swap_option)]
    #[serde(rename = "width")]
    pub window_controls_width: Option<u32>,
    #[clap(long, help = "[default: 40] Height of window title/controls bar")]
    #[merge(strategy = swap_option)]
    #[serde(rename = "height")]
    pub window_controls_height: Option<u32>,
    #[clap(long, help = "[default: 10] Padding of title on window bar")]
    #[merge(strategy = swap_option)]
    #[serde(rename = "padding")]
    pub titlebar_padding: Option<u32>,
}

impl From<GenerationSettingsArgs> for GenerationSettings {
    fn from(val: GenerationSettingsArgs) -> Self {
        let shadow_color = val
            .colors
            .clone()
            .shadow_color
            .map(|b| Background::try_from(b).unwrap_or_default())
            .unwrap_or_default();

        GenerationSettings {
            copy: val.copy,
            output: val.output.clone(),
            save_format: val.save_format.clone(),
            colors: val.colors.into(),
            padding: (val.padding_x.unwrap_or(80), val.padding_y.unwrap_or(100)),
            round_corner: val.radius.or(Some(15)),
            shadow: val.shadow.then_some(Shadow {
                shadow_color,
                use_inner_image: val.shadow_image,
                blur_radius: val.shadow_blur.unwrap_or(50.),
            }),
            fonts: val.fonts.unwrap_or_default(),
            author: val.author.clone(),
            author_font: val.author_font.clone().unwrap_or("Hack".to_string()),
            window_controls: val.window_controls.into(),
        }
    }
}

impl From<ColorsArgs> for Colors {
    fn from(val: ColorsArgs) -> Self {
        let background = val
            .background
            .map(|b| {
                Background::try_from(b)
                    .unwrap_or(Background::Solid(image::Rgba([0x32, 0x32, 0x32, 255])))
            })
            .unwrap_or(Background::Solid(image::Rgba([0x32, 0x32, 0x32, 255])));
        let windows_background = val
            .window_background
            .map(|b| {
                Background::try_from(b).unwrap_or(Background::Solid(Rgba([0x42, 0x87, 0xf5, 255])))
            })
            .unwrap_or(Background::Solid(Rgba([0x42, 0x87, 0xf5, 255])));
        Colors {
            background,
            windows_background,
            author_color: val
                .author_color
                .unwrap_or("#FFFFFF".to_string())
                .to_rgba()
                .unwrap(),
            windows_title: val
                .window_title_color
                .unwrap_or("#FFFFFF".to_string())
                .to_rgba()
                .unwrap(),
        }
    }
}

impl From<WindowControlsArgs> for WindowControls {
    fn from(val: WindowControlsArgs) -> Self {
        WindowControls {
            enable: val.enable,
            title: val.window_title.clone(),
            width: val.window_controls_width.unwrap_or(120),
            height: val.window_controls_height.unwrap_or(40),
            title_padding: val.titlebar_padding.unwrap_or(10),
        }
    }
}
