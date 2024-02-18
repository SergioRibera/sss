use clap::{Parser, ValueEnum};
use merge2::Merge;
use serde::{Deserialize, Serialize};

use crate::{PADDING, SIZE};

#[derive(Default, Clone, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum Position {
    Left,
    Right,
    Top,
    #[default]
    Bottom,
}

#[derive(Clone, Default, Debug, Deserialize, Merge, Parser, Serialize)]
#[clap(version, author)]
pub struct Config {
    #[clap(long, short = 'l')]
    #[merge(strategy = swap_option)]
    /// Location for the launcher panel
    pub position: Option<Position>,
    /// Command to be executed at launcher startup (useful to run applications like satty)
    #[clap(long, short)]
    #[merge(strategy = swap_option)]
    pub pre_command: Option<String>,
    /// Command to launch sss
    #[clap(
        long,
        short = 'r',
        default_value = "sh -c 'sss --area \"$(slurp -d)\"'"
    )]
    #[merge(strategy = swap_option)]
    pub area_command: Option<String>,
    /// Command to launch sss
    #[clap(long, short, default_value = "sh -c 'sss --screen --current")]
    #[merge(strategy = swap_option)]
    pub screen_command: Option<String>,
    /// Command to launch sss
    #[clap(long, short, default_value = "sh -c 'sss --screen")]
    #[merge(strategy = swap_option)]
    pub all_command: Option<String>,
}

#[inline]
fn swap_option<T>(left: &mut Option<T>, right: &mut Option<T>) {
    if left.is_none() || right.is_some() {
        core::mem::swap(left, right);
    }
}

pub fn get_config() -> Config {
    let config_path = directories::BaseDirs::new()
        .unwrap()
        .config_dir()
        .join("sss");

    let _ = std::fs::create_dir_all(config_path.clone());

    let config_path = config_path.join("launcher.toml");
    let mut args = Config::parse();

    std::fs::read_to_string(config_path)
        .map(|cfg_content| {
            let mut config: Config = toml::from_str(&cfg_content).unwrap();
            config.merge(&mut args);
            config
        })
        .unwrap_or(args)
}

impl Config {
    pub fn get_count(&self) -> f32 {
        let mut n = 0.;

        if self.all_command.is_some() {
            n += 1.;
        }
        if self.area_command.is_some() {
            n += 1.;
        }
        if self.screen_command.is_some() {
            n += 1.;
        }

        n
    }

    pub fn get_size(&self) -> (f32, f32) {
        let n = self.get_count();
        self.position
            .as_ref()
            .map(|pos| match pos {
                Position::Top | Position::Bottom => (SIZE * PADDING * n, SIZE * PADDING * 2.),
                _ => (SIZE * PADDING * 2., SIZE * PADDING * n),
            })
            .unwrap_or((SIZE * PADDING * n, SIZE * PADDING * 2.))
    }
}
