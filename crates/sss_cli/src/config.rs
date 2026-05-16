use std::path::PathBuf;

use clap::Parser;
use merge2::{bool::overwrite_false, option::recursive, Merge};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use sss_lib::{default_bool, swap_option};

use crate::error::Configuration as ConfigurationError;
use crate::{str_to_area, Area};

/// Sentinel used by clap's `default_missing_value` to mark "flag present
/// without a value" — the parser turns it into the `Interactive` variant.
const INTERACTIVE_SENTINEL: &str = "__interactive__";

// --------------------------------------------------------------------------
// AreaSpec  /  ScreenSpec  /  WindowSpec
// --------------------------------------------------------------------------

/// Source of a target area.
#[derive(Clone, Debug)]
pub enum AreaSpec {
    /// `--area` without a value: open the interactive area selector.
    Interactive,
    /// `--area "x,y WxH"`: capture this rectangle directly.
    Direct(Area),
}

fn parse_area_spec(s: &str) -> Result<AreaSpec, String> {
    if s == INTERACTIVE_SENTINEL {
        Ok(AreaSpec::Interactive)
    } else {
        str_to_area(s).map(AreaSpec::Direct)
    }
}

impl<'de> Deserialize<'de> for AreaSpec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        parse_area_spec(&String::deserialize(d)?).map_err(D::Error::custom)
    }
}
impl Serialize for AreaSpec {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            AreaSpec::Interactive => "interactive".serialize(s),
            AreaSpec::Direct(a) => a.serialize(s),
        }
    }
}

/// Source of a target screen (monitor).
#[derive(Clone, Debug)]
pub enum ScreenSpec {
    /// `--screen-id` without a value: open the monitor selector.
    Interactive,
    /// `--screen-id <id|name>`: pick this monitor directly.
    Direct(String),
}

fn parse_screen_spec(s: &str) -> Result<ScreenSpec, String> {
    if s == INTERACTIVE_SENTINEL {
        Ok(ScreenSpec::Interactive)
    } else {
        Ok(ScreenSpec::Direct(s.to_string()))
    }
}

impl<'de> Deserialize<'de> for ScreenSpec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(ScreenSpec::Direct(s))
    }
}
impl Serialize for ScreenSpec {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            ScreenSpec::Interactive => "interactive".serialize(s),
            ScreenSpec::Direct(v) => v.serialize(s),
        }
    }
}

/// Source of a target window.
#[derive(Clone, Debug)]
pub enum WindowSpec {
    /// `--window` without a value: open the window picker.
    Interactive,
    /// `--window <id|title>`: pick this window directly. Numeric values are
    /// treated as IDs, otherwise as a title substring.
    Direct(String),
}

fn parse_window_spec(s: &str) -> Result<WindowSpec, String> {
    if s == INTERACTIVE_SENTINEL {
        Ok(WindowSpec::Interactive)
    } else {
        Ok(WindowSpec::Direct(s.to_string()))
    }
}

impl<'de> Deserialize<'de> for WindowSpec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(WindowSpec::Direct(String::deserialize(d)?))
    }
}
impl Serialize for WindowSpec {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            WindowSpec::Interactive => "interactive".serialize(s),
            WindowSpec::Direct(v) => v.serialize(s),
        }
    }
}

// --------------------------------------------------------------------------
// CLI / config
// --------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize, Merge, Parser, Serialize)]
#[clap(version, author)]
#[serde(rename_all = "kebab-case")]
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
#[serde(rename_all = "kebab-case")]
pub struct CliConfig {
    #[clap(
        long,
        help = "When you take from a screen or window, capture the one on which the mouse is located."
    )]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub current: bool,

    #[clap(
        long,
        help = "Composite the mouse cursor into the captured frame (where the backend supports it)"
    )]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub show_cursor: bool,

    /// When present without a value, opens the interactive monitor selector.
    /// When combined with `--current`, captures the monitor under the
    /// cursor directly (legacy behaviour).
    #[clap(
        long,
        help = "Open the monitor selector (or capture the current monitor when combined with --current)."
    )]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub screen: bool,

    #[clap(
        long,
        num_args = 0..=1,
        default_missing_value = INTERACTIVE_SENTINEL,
        value_parser = parse_screen_spec,
        help = "Pick a screen. Without a value, opens the monitor selector. \
                With a value, picks the monitor by id or name directly."
    )]
    #[merge(strategy = swap_option)]
    pub screen_id: Option<ScreenSpec>,

    #[clap(
        long,
        num_args = 0..=1,
        default_missing_value = INTERACTIVE_SENTINEL,
        value_parser = parse_area_spec,
        help = "Pick an area. Without a value, opens the interactive area selector. \
                With a value (\"x,y WxH\"), captures the given rectangle directly."
    )]
    #[merge(strategy = swap_option)]
    pub area: Option<AreaSpec>,

    #[clap(
        long,
        num_args = 0..=1,
        default_missing_value = INTERACTIVE_SENTINEL,
        value_parser = parse_window_spec,
        help = "Pick a window. Without a value, opens the window picker. \
                With a value, picks the window by id (numeric) or by title substring."
    )]
    #[merge(strategy = swap_option)]
    pub window: Option<WindowSpec>,

    #[clap(
        long,
        help = "Force the interactive selector even when targeting flags carry an explicit value."
    )]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub interactive: bool,

    #[clap(
        long,
        help = "Hide the annotation toolbar in interactive mode (slurp-class picker only)."
    )]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub no_toolbar: bool,

    /// Force a specific capture backend. Useful for diagnosing why
    /// auto-detection picked the wrong one (e.g. `--capture-backend wayland`
    /// to skip the portal fallback on wlroots compositors).
    #[clap(
        long,
        value_parser = parse_backend,
        help = "Force a capture backend: auto | wayland | portal | x11 | windows | macos"
    )]
    #[merge(strategy = swap_option)]
    pub capture_backend: Option<BackendChoice>,

    /// Bump the default log level to `info` (warnings + backend info).
    #[clap(long, short = 'v')]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub verbose: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum BackendChoice {
    Auto,
    Wayland,
    Portal,
    X11,
    Windows,
    MacOs,
}

fn parse_backend(s: &str) -> Result<BackendChoice, String> {
    match s.to_lowercase().as_str() {
        "auto" => Ok(BackendChoice::Auto),
        "wayland" | "wayland-wlr" | "wlr" => Ok(BackendChoice::Wayland),
        "portal" | "wayland-portal" => Ok(BackendChoice::Portal),
        "x11" | "xorg" => Ok(BackendChoice::X11),
        "windows" | "win32" | "win" => Ok(BackendChoice::Windows),
        "macos" | "mac" => Ok(BackendChoice::MacOs),
        other => Err(format!(
            "unknown backend {other:?}; expected auto|wayland|portal|x11|windows|macos"
        )),
    }
}

impl BackendChoice {
    pub fn to_kind(self) -> sss_capture::BackendKind {
        use sss_capture::BackendKind as K;
        match self {
            BackendChoice::Auto => K::Auto,
            BackendChoice::Wayland => K::Wayland,
            BackendChoice::Portal => K::WaylandPortal,
            BackendChoice::X11 => K::X11,
            BackendChoice::Windows => K::WindowsGdi,
            BackendChoice::MacOs => K::MacOS,
        }
    }
}

impl<'de> Deserialize<'de> for BackendChoice {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        parse_backend(&String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}
impl Serialize for BackendChoice {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let name = match self {
            BackendChoice::Auto => "auto",
            BackendChoice::Wayland => "wayland",
            BackendChoice::Portal => "portal",
            BackendChoice::X11 => "x11",
            BackendChoice::Windows => "windows",
            BackendChoice::MacOs => "macos",
        };
        name.serialize(s)
    }
}

impl CliConfig {
    /// Is the user asking for a direct (non-interactive) capture?
    ///
    /// Returns the [`SelectorMode`]-equivalent target only if every targeting
    /// flag is either absent or carries a direct value.
    pub fn direct_target(&self) -> Option<DirectTarget> {
        // The legacy "--screen --current" combination still bypasses the
        // selector: it has a perfectly clear semantic ("the monitor under
        // the cursor").
        if self.screen && self.current && self.area.is_none() && self.window.is_none() {
            return Some(DirectTarget::CurrentMonitor);
        }
        // --area "x,y WxH" → direct
        if let Some(AreaSpec::Direct(a)) = &self.area {
            return Some(DirectTarget::Area(*a));
        }
        // --screen-id <v> → direct
        if let Some(ScreenSpec::Direct(v)) = &self.screen_id {
            return Some(DirectTarget::Screen(v.clone()));
        }
        // --window <v> → direct
        if let Some(WindowSpec::Direct(v)) = &self.window {
            return Some(DirectTarget::Window(v.clone()));
        }
        None
    }
}

/// Outcome of resolving the CLI flags before opening the selector.
#[derive(Clone, Debug)]
pub enum DirectTarget {
    /// `--screen --current`
    CurrentMonitor,
    /// `--area "x,y WxH"`
    Area(Area),
    /// `--screen-id <id|name>`
    Screen(String),
    /// `--window <id|title>`
    Window(String),
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
