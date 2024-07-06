use std::path::PathBuf;

use clap::Parser;
use merge2::{bool::overwrite_false, option::recursive, Merge};
use serde::{Deserialize, Serialize};
use sss_lib::{default_bool, swap_option};

use crate::error::Configuration as ConfigurationError;
use crate::{str_to_area, Area};

#[derive(Clone, Debug, Deserialize, Merge, Parser, Serialize)]
#[clap(version, author)]
struct ClapConfig {
    #[clap(long, help = "Set custom config file path")]
    #[serde(skip)]
    #[merge(skip)]
    config: Option<PathBuf>,
    #[clap(flatten)]
    #[merge(strategy = recursive)]
    pub cli: Option<CliConfig>,
    // lib configs
    #[clap(flatten)]
    #[serde(rename = "general")]
    pub lib_config: sss_lib::GenerationSettingsArgs,
}

#[derive(Clone, Debug, Default, Deserialize, Merge, Parser, Serialize)]
pub struct CliConfig {
    #[clap(
        long,
        help = "When you take from a screen or window, capture the one on which the mouse is located."
    )]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub current: bool,
    #[clap(long, help = "Capture cursor (Only Wayland)")]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub show_cursor: bool,
    #[clap(long, help = "Capture a full screen")]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub screen: bool,
    #[clap(long, help = "ID or Name of screen to capture")]
    #[merge(strategy = swap_option)]
    pub screen_id: Option<String>,
    #[clap(long, help = "Captures an area of the screen", value_parser = str_to_area)]
    #[merge(strategy = swap_option)]
    pub area: Option<Area>,
}

pub fn get_config() -> Result<(CliConfig, sss_lib::GenerationSettings), ConfigurationError> {
    let mut args = ClapConfig::parse();

    let config_path = if let Some(path) = args.config.as_ref() {
        tracing::trace!("Loading custom path");
        path.clone()
    } else {
        let config_path = directories::BaseDirs::new()
            .ok_or(ConfigurationError::InvalidHome)?
            .config_dir()
            .join("sss");

        let _ = std::fs::create_dir_all(config_path.clone());

        tracing::trace!("Loading global config");
        config_path.join("config.toml")
    };
    tracing::info!("Reading configs from path: {config_path:?}");

    if let Ok(cfg_content) = std::fs::read_to_string(config_path) {
        tracing::debug!("Merging from config file");
        let mut config: ClapConfig = toml::from_str(&cfg_content)?;
        config.merge(&mut args);
        return Ok((config.cli.unwrap_or_default(), config.lib_config.into()));
    }
    Ok((args.cli.unwrap_or_default(), args.lib_config.into()))
}
