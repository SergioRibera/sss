use std::ops::Range;
use std::path::PathBuf;

use clap::Parser;
use clap_stdin::FileOrStdin;
use merge2::{bool::overwrite_false, option::recursive, Merge};
use serde::{Deserialize, Serialize};
use sss_lib::{default_bool, swap_option};

use crate::error::CodeScreenshotError;

#[derive(Clone, Debug, Deserialize, Merge, Parser, Serialize)]
#[clap(author, version, about)]
struct ClapConfig {
    #[clap(long, help = "Set custom config file path")]
    #[serde(skip)]
    #[merge(skip)]
    config: Option<PathBuf>,
    #[clap(flatten)]
    #[merge(strategy = recursive)]
    pub code: Option<CodeConfig>,
    // lib configs
    #[clap(flatten)]
    #[serde(rename = "general")]
    pub lib_config: sss_lib::GenerationSettingsArgs,
}

#[derive(Clone, Debug, Deserialize, Merge, Parser, Serialize)]
pub struct CodeConfig {
    #[clap(help = "Content to take screenshot. It accepts stdin or File")]
    #[serde(skip)]
    #[merge(strategy = swap_option)]
    pub content: Option<FileOrStdin<String>>,
    #[clap(long, help = "Generate cache of theme and/or syntaxes")]
    #[serde(skip)]
    #[merge(strategy = swap_option)]
    pub build_cache: Option<PathBuf>,
    #[clap(
        long,
        short,
        help = "Theme file to use. May be a path, or an embedded theme. Embedded themes will take precendence."
    )]
    #[merge(strategy = swap_option)]
    pub theme: Option<String>,
    #[clap(
        long,
        help = "[Not recommended for manual use] Set theme from vim highlights, format: group,bg,fg,style;group,bg,fg,style;"
    )]
    #[merge(strategy = swap_option)]
    pub vim_theme: Option<String>,
    // Setting synctect
    #[clap(
        long,
        short = 'l',
        conflicts_with_all = &[ "list_themes", "output" ],
        help = "Lists supported file types"
    )]
    #[merge(strategy = overwrite_false)]
    #[serde(skip)]
    pub list_file_types: bool,
    #[clap(
        long,
        short = 'L', 
        conflicts_with = "output",
        help = "Lists themes"
    )]
    #[merge(strategy = overwrite_false)]
    #[serde(skip)]
    pub list_themes: bool,
    #[clap(
        long,
        help = "Additional folder to search for .sublime-syntax files in"
    )]
    #[merge(strategy = swap_option)]
    pub extra_syntaxes: Option<String>,
    #[clap(long, short, help = "Set the extension of language input")]
    #[serde(skip)]
    pub extension: Option<String>,
    // Render options
    #[clap(
        long,
        help = "[default: #323232] Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path"
    )]
    #[merge(strategy = swap_option)]
    pub code_background: Option<String>,
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
    #[clap(long, help = "Tab width")]
    #[merge(strategy = swap_option)]
    pub tab_width: Option<u8>,
}

impl Default for CodeConfig {
    fn default() -> Self {
        Self {
            content: None,
            build_cache: None,
            theme: Some("base16-ocean.dark".to_string()),
            vim_theme: None,
            list_file_types: false,
            list_themes: false,
            extra_syntaxes: None,
            extension: None,
            code_background: Some("#323232".to_string()),
            lines: Some(Range {
                start: 0,
                end: usize::MAX,
            }),
            highlight_lines: Some(Range {
                start: 0,
                end: usize::MAX,
            }),
            line_numbers: true,
            tab_width: Some(4),
        }
    }
}

pub fn get_config() -> (CodeConfig, sss_lib::GenerationSettings) {
    let mut args = ClapConfig::parse();

    let config_path = if let Some(path) = args.config.as_ref() {
        path.clone()
    } else {
        let config_path = directories::BaseDirs::new()
            .unwrap()
            .config_dir()
            .join("sss");

        let _ = std::fs::create_dir_all(config_path.clone());

        config_path.join("config.toml")
    };
    // println!("Reading configs from path: {config_path:?}");

    if let Ok(cfg_content) = std::fs::read_to_string(config_path) {
        // println!("Merging from config file");
        let mut config: ClapConfig = toml::from_str(&cfg_content).unwrap();
        config.merge(&mut args);
        return (config.code.unwrap_or_default(), config.lib_config.into());
    }
    (args.code.unwrap_or_default(), args.lib_config.into())
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
            .unwrap_or(usize::MAX),
    );

    Ok(Range { start, end })
}
