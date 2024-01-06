use std::ops::Range;
use std::path::PathBuf;

use clap::Parser;
use clap_stdin::FileOrStdin;
use sss_lib::{GenerationSettings, Shadow};

use crate::error::CodeScreenshotError;

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
    #[clap(long, help = "Lines range to take screenshot, format start..end", value_parser=parse_range)]
    pub lines: Option<Range<usize>>,
    #[clap(long, help = "Lines to highlight over the rest, format start..end", value_parser=parse_range)]
    pub highlight_lines: Option<Range<usize>>,
}

pub fn get_config() -> CodeConfig {
    CodeConfig::parse()
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
            .unwrap_or_default(),
        end_str
            .replace(other, "")
            .parse::<usize>()
            .unwrap_or(usize::MAX),
    );

    Ok(Range { start, end })
}

impl Into<GenerationSettings> for CodeConfig {
    fn into(self) -> GenerationSettings {
        GenerationSettings {
            background: (),
            padding: (),
            round_corner: (),
            shadow: Some(Shadow {
                background: (),
                use_inner_image: (),
                shadow_color: (),
                blur_radius: (),
            }),
        }
    }
}
