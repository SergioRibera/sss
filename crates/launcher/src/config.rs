use clap::Parser;
use merge2::{bool::overwrite_false, Merge};
use serde::{Deserialize, Serialize};
use sss_lib::{default_bool, swap_option};

use crate::{str_to_area, Area};

#[derive(Clone, Debug, Deserialize, Merge, Parser, Serialize)]
#[clap(version, author)]
struct ClapConfig {
    #[clap(flatten)]
    #[merge(strategy = swap_option)]
    pub cli: Option<CliConfig>,
    // lib configs
    #[clap(flatten)]
    #[serde(rename = "general")]
    pub lib_config: sss_lib::GenerationSettingsArgs,
}
