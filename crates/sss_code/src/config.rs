use std::ops::Range;
use std::path::PathBuf;

use clap::Parser;
use clap_stdin::FileOrStdin;
use sss_lib::image::error::{ImageFormatHint, UnsupportedError, UnsupportedErrorKind};
use sss_lib::image::{ImageError, ImageFormat};
use sss_lib::{Background, GenerationSettings, Shadow, ToRgba};

use crate::error::CodeScreenshotError;

pub type FontList = Vec<(String, f32)>;

#[derive(Clone, Parser)]
#[clap(author, version, about)]
pub struct CodeConfig {
    #[clap(help = "Content to take screenshot. It accepts stdin or File")]
    pub content: Option<FileOrStdin<String>>,
    #[clap(
        long,
        short,
        default_value = "base16-ocean.dark",
        help = "Theme file to use. May be a path, or an embedded theme. Embedded themes will take precendence."
    )]
    pub theme: String,
    #[clap(long, help = "[default: Hack=26.0;] The font used to render, format: Font Name=size;Other Font Name=12.0", value_parser = parse_font_str)]
    pub font: Option<FontList>,
    #[clap(
        long,
        help = "[Not recommended for manual use] Set theme from vim highlights, format: group,bg,fg,style;group,bg,fg,style;"
    )]
    pub vim_theme: Option<String>,
    // Setting synctect
    #[clap(long, short = 'l', help = "Lists supported file types")]
    pub list_file_types: bool,
    #[clap(long, short = 'L', help = "Lists themes")]
    pub list_themes: bool,
    #[clap(
        long,
        help = "Additional folder to search for .sublime-syntax files in"
    )]
    pub extra_syntaxes: Option<PathBuf>,
    #[clap(long, short, help = "Set the extension of language input")]
    pub extension: Option<String>,
    // Render options
    #[clap(long, default_value="..", help = "Lines range to take screenshot, format start..end", value_parser=parse_range)]
    pub lines: Option<Range<usize>>,
    #[clap(long, default_value="..", help = "Lines to highlight over the rest, format start..end", value_parser=parse_range)]
    pub highlight_lines: Option<Range<usize>>,
    #[clap(long, short = 'n', help = "Show Line numbers")]
    pub line_numbers: bool,
    #[clap(long, default_value = "4", help = "Tab width")]
    pub tab_width: u8,
    #[clap(long, help = "Whether show the window controls")]
    pub window_controls: bool,
    #[clap(long, help = "Window title")]
    pub window_title: Option<String>,
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
    #[clap(long, help = "Generate shadow from inner image")]
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

pub fn get_config() -> CodeConfig {
    CodeConfig::parse()
}

impl From<CodeConfig> for GenerationSettings {
    fn from(val: CodeConfig) -> Self {
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

fn str_to_format(s: &str) -> Result<ImageFormat, ImageError> {
    ImageFormat::from_extension(s).ok_or(ImageError::Unsupported(
        UnsupportedError::from_format_and_kind(
            ImageFormatHint::Name(s.to_string()),
            UnsupportedErrorKind::Format(ImageFormatHint::Name(s.to_string())),
        ),
    ))
}

fn parse_font_str(s: &str) -> Result<Vec<(String, f32)>, String> {
    Ok(s.split(';')
        .map(|font| {
            let (name, size) = font.split_once('=').unwrap();
            (name.to_owned(), size.parse::<f32>().unwrap_or(26.))
        })
        .collect::<Vec<(String, f32)>>())
}
