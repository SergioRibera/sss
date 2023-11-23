use std::ops::Range;
use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[clap(author, version, about)]
pub struct AppArgs {
    #[clap(long, short, help = "File to take screenshot")]
    pub file: Option<PathBuf>,
    #[clap(long, short, help = "Theme for highlight")]
    pub theme: Option<String>,
    #[clap(long, short, help = "Lines range to take screenshot", value_parser=parse_range)]
    pub lines: Option<Range<u32>>,
}

fn parse_range(s: &str) -> Result<Range<u32>, String> {
    let Some(other) = s.chars().find(|c| !c.is_numeric()) else {
        return Err("The format for range are start..end".to_string());
    };

    let Some((start_str, end_str)) = s.split_once(&other.to_string()) else {
        return Err("The format for range are start..end".to_string());
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
