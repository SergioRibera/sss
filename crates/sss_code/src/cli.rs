use std::ops::Range;
use std::path::PathBuf;

use clap::Parser;

use crate::error::CodeScreenshotError;

pub fn get_args() -> AppArgs {
    AppArgs::parse()
}

#[derive(Parser)]
#[clap(author, version, about)]
pub struct AppArgs {
    #[clap(long, short, help = "File to take screenshot", default_value = "false")]
    pub print: bool,
    #[clap(long, short, help = "File to take screenshot")]
    pub file: Option<PathBuf>,
    #[clap(long, short, help = "Theme for highlight")]
    pub theme: Option<String>,
    #[clap(long, short, help = "Lines range to take screenshot", value_parser=parse_range)]
    pub lines: Option<Range<u32>>,
}

fn parse_range(s: &str) -> Result<Range<u32>, CodeScreenshotError> {
    let Some(other) = s.chars().find(|c| !c.is_numeric()) else {
        return Err(CodeScreenshotError::InvalidFormat("range", "start..end"));
    };

    let Some((start_str, end_str)) = s.split_once(&other.to_string()) else {
        return Err(CodeScreenshotError::InvalidFormat("range", "start..end"));
    };

    let (start, end) = (
        start_str
            .replace(other, "")
            .parse::<u32>()
            .unwrap_or_default(),
        end_str
            .replace(other, "")
            .parse::<u32>()
            .unwrap_or(u32::MAX),
    );

    Ok(Range { start, end })
}
