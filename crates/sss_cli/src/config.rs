use clap::Parser;
use merge2::{bool::overwrite_false, Merge};
use serde::{Deserialize, Serialize};
use sss_lib::font::parse_font_str;
use sss_lib::font::FontCollection;
use sss_lib::{Background, GenerationSettings, Shadow, ToRgba};

use crate::{str_to_area, Area};

const fn default_bool() -> bool {
    false
}

#[inline]
fn swap_option<T>(left: &mut Option<T>, right: &mut Option<T>) {
    if left.is_none() || right.is_some() {
        core::mem::swap(left, right);
    }
}

#[derive(Clone, Debug, Default, Deserialize, Merge, Parser, Serialize)]
#[clap(version, author)]
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
    // Screenshot Section
    #[clap(
        long,
        help = "[default: Hack=12.0;] The font used to render, format: Font Name=size;Other Font Name=12.0",
        value_parser = parse_font_str
    )]
    #[merge(strategy = swap_option)]
    pub fonts: Option<FontCollection>,
    #[clap(
        long,
        short,
        help = "[default: #323232] Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path"
    )]
    #[merge(strategy = swap_option)]
    pub background: Option<String>,
    #[clap(long, short, help = "[default: 15] ")]
    #[merge(strategy = swap_option)]
    pub radius: Option<u32>,
    #[clap(long, help = "Author Name of screenshot")]
    #[merge(strategy = swap_option)]
    pub author: Option<String>,
    #[clap(long, help = "[default: #FFFFFF] Title bar text color")]
    #[merge(strategy = swap_option)]
    pub author_color: Option<String>,
    #[clap(long, help = "[default: Hack] Font to render Author")]
    #[merge(strategy = swap_option)]
    pub author_font: Option<String>,
    // Window Bar
    #[clap(long, help = "Whether show the window controls")]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub window_controls: bool,
    #[clap(long, help = "Window title")]
    #[merge(strategy = swap_option)]
    pub window_title: Option<String>,
    #[clap(long, help = "[default: #4287f5] Window bar background")]
    #[merge(strategy = swap_option)]
    pub window_background: Option<String>,
    #[clap(long, help = "[default: #FFFFFF] Title bar text color")]
    #[merge(strategy = swap_option)]
    pub window_title_color: Option<String>,
    #[clap(long, help = "[default 120] Width of window controls")]
    #[merge(strategy = swap_option)]
    pub window_controls_width: Option<u32>,
    #[clap(long, help = "[default: 40] Height of window title/controls bar")]
    #[merge(strategy = swap_option)]
    pub window_controls_height: Option<u32>,
    #[clap(long, help = "[default: 10] Padding of title on window bar")]
    #[merge(strategy = swap_option)]
    pub titlebar_padding: Option<u32>,
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
    #[clap(
        long,
        help = "[default: #707070] Support: '#RRGGBBAA' 'h;#RRGGBBAA;#RRGGBBAA' 'v;#RRGGBBAA;#RRGGBBAA' or file path"
    )]
    #[merge(strategy = swap_option)]
    pub shadow_color: Option<String>,
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
        help = "If it is set then the result will be saved here, otherwise it will not be saved."
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
}

pub fn get_config() -> CliConfig {
    let config_path = directories::BaseDirs::new()
        .unwrap()
        .config_dir()
        .join("sss");

    let _ = std::fs::create_dir_all(config_path.clone());

    let config_path = config_path.join("config.toml");
    println!("Reading configs from path: {config_path:?}");

    if let Ok(cfg_content) = std::fs::read_to_string(config_path) {
        println!("Merging from config file");
        let mut config: CliConfig = toml::from_str(&cfg_content).unwrap();
        let mut args = CliConfig::parse();

        config.merge(&mut args);
        return config;
    }
    CliConfig::parse()
}

impl From<CliConfig> for GenerationSettings {
    fn from(val: CliConfig) -> Self {
        let background = Background::try_from(val.background.unwrap().clone()).unwrap();
        let windows_background =
            Background::try_from(val.window_background.unwrap().clone()).unwrap();
        let shadow_color = Background::try_from(val.shadow_color.unwrap().clone()).unwrap();

        GenerationSettings {
            windows_background,
            background,
            padding: (val.padding_x.unwrap(), val.padding_y.unwrap()),
            round_corner: val.radius,
            shadow: val.shadow.then_some(Shadow {
                shadow_color,
                use_inner_image: val.shadow_image,
                blur_radius: val.shadow_blur.unwrap(),
            }),
            fonts: val.fonts.unwrap_or_default(),
            author: val.author.clone(),
            author_font: val.author_font.clone().unwrap(),
            author_color: val.author_color.unwrap().to_rgba().unwrap(),
            window_controls: val.window_controls,
            windows_title: val.window_title.clone(),
            windows_title_color: val.window_title_color.unwrap().to_rgba().unwrap(),
            window_controls_width: val.window_controls_width.unwrap(),
            window_controls_height: val.window_controls_height.unwrap(),
            titlebar_padding: val.titlebar_padding.unwrap(),
        }
    }
}
