use std::path::PathBuf;

use clap::Parser;
use merge2::{bool::overwrite_false, option::recursive, Merge};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use sss_capture_ui::UiConfig;
use sss_lib::config_loader::{load_with_imports, HasImports, LoadError};
use sss_lib::{default_bool, swap_option, RootArgs};
#[cfg(feature = "ocr")]
use sss_ocr::{GpuMode, Language, Tier};

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
    #[clap(flatten)]
    #[serde(flatten)]
    pub root: RootArgs,
    #[clap(flatten)]
    #[serde(default)]
    #[merge(strategy = recursive)]
    pub cli: Option<CliConfig>,
    #[clap(flatten)]
    #[serde(rename = "general", default)]
    pub lib_config: sss_lib::GenerationSettingsArgs,
    /// Configuration block for the interactive selector / annotation UI.
    /// Loaded from `[capture-ui]` in `config.toml`; not exposed as
    /// individual CLI flags (the surface is too wide to be ergonomic on
    /// the command line — use the config file).
    #[clap(skip)]
    #[serde(default, rename = "capture-ui")]
    #[merge(strategy = swap_option)]
    pub capture_ui: Option<UiConfig>,
    #[cfg(feature = "ocr")]
    #[clap(flatten)]
    #[serde(default, rename = "ocr")]
    #[merge(strategy = recursive)]
    pub ocr: Option<OcrConfig>,
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

    /// Persist the last interactive area selection and pre-seed the
    /// selector with it next time `--area` is opened without a value.
    /// Stored under `${XDG_CONFIG_HOME}/sss/last_selection.toml`.
    #[clap(long, help = "Remember the last interactive area selection.")]
    #[merge(strategy = overwrite_false)]
    #[serde(default = "default_bool")]
    pub remember_last_selection: bool,
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

// --------------------------------------------------------------------------
// [ocr] section
// --------------------------------------------------------------------------

/// Configuration block for the OCR engine. Loaded from `[ocr]` in
/// `config.toml`; the user-facing `--ocr [true|false]` flag overrides
/// `enable` from the command line.
///
/// Everything except `enable` lives in the config file only — the surface
/// (tier, language list, model overrides) is wider than what's pleasant on
/// the command line and these settings rarely change between captures.
#[cfg(feature = "ocr")]
#[derive(Clone, Debug, Default, Deserialize, Merge, Parser, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct OcrConfig {
    /// Run the OCR pipeline after every capture. When false the selector
    /// behaves exactly as the OCR-less build did.
    #[clap(
        id = "ocr-enable",
        long = "ocr",
        value_name = "BOOL",
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = clap::builder::BoolishValueParser::new(),
        help = "Enable or disable the OCR pipeline for this run (true|false)."
    )]
    #[merge(strategy = swap_option)]
    pub enable: Option<bool>,

    /// Picks model sizes against host hardware. `auto` (default) chooses
    /// between light/standard/heavy at startup.
    #[clap(skip)]
    #[serde(default = "default_tier")]
    #[merge(strategy = overwrite_tier)]
    pub tier: Tier,

    /// Recognition languages to pre-download. The first entry is the
    /// active one at runtime; the rest stay cached for fast switching.
    /// Accepts both ISO 639-1 codes (`"en"`, `"es"`, `"ja"`) and the
    /// PaddleOCR script names (`"latin"`, `"cyrillic"`, `"arabic"`).
    /// Defaults to `["auto"]`.
    #[clap(skip)]
    #[serde(default = "default_languages")]
    #[merge(strategy = merge_languages)]
    pub language: Vec<String>,

    /// Opt into the formula recognition model. Only honoured at
    /// [`Tier::Heavy`] — at lighter tiers the formula model is skipped.
    #[clap(skip)]
    #[serde(default = "default_bool")]
    #[merge(strategy = overwrite_false)]
    pub formula: bool,

    /// Override the on-disk cache directory. When empty (the default) the
    /// OCR worker uses `$XDG_DATA_HOME/sss/models` (or the equivalent on
    /// macOS / Windows via [`directories`]).
    #[clap(skip)]
    #[serde(default)]
    #[merge(strategy = swap_option)]
    pub models_dir: Option<PathBuf>,

    /// Execution provider for ORT inference. `auto` (default) picks the
    /// best provider compiled into the binary for the host — falls back
    /// to CPU when no GPU EP is available. Explicit values force the
    /// pipeline onto a specific backend; ORT still falls back to CPU at
    /// runtime if the EP isn't actually present in the loaded
    /// `libonnxruntime`.
    #[clap(
        long = "ocr-gpu",
        value_name = "MODE",
        default_value = "auto",
        value_parser = parse_gpu_mode,
        help = "OCR execution provider: auto, cpu, cuda, tensorrt, coreml, directml, openvino, webgpu."
    )]
    #[serde(default = "default_gpu")]
    #[merge(strategy = overwrite_gpu)]
    pub gpu: GpuMode,
}

#[cfg(feature = "ocr")]
fn default_gpu() -> GpuMode {
    GpuMode::Auto
}

#[cfg(feature = "ocr")]
fn parse_gpu_mode(s: &str) -> Result<GpuMode, String> {
    match s.to_ascii_lowercase().as_str() {
        "auto" => Ok(GpuMode::Auto),
        "cpu" => Ok(GpuMode::Cpu),
        "cuda" => Ok(GpuMode::Cuda),
        "tensorrt" | "tensor-rt" | "trt" => Ok(GpuMode::TensorRT),
        "coreml" | "core-ml" => Ok(GpuMode::CoreML),
        "directml" | "direct-ml" | "dml" => Ok(GpuMode::DirectML),
        "openvino" | "open-vino" => Ok(GpuMode::OpenVino),
        "webgpu" | "web-gpu" => Ok(GpuMode::WebGpu),
        other => Err(format!(
            "unknown GPU mode '{other}'; expected one of: auto, cpu, cuda, tensorrt, coreml, directml, openvino, webgpu"
        )),
    }
}

/// Preserve a non-`Auto` GPU mode from one side; `Auto` never overrides
/// an explicit choice from the other layer.
#[cfg(feature = "ocr")]
fn overwrite_gpu(dst: &mut GpuMode, src: &mut GpuMode) {
    if !matches!(*src, GpuMode::Auto) {
        *dst = *src;
    }
}

#[cfg(feature = "ocr")]
fn default_tier() -> Tier {
    Tier::Auto
}

#[cfg(feature = "ocr")]
fn default_languages() -> Vec<String> {
    vec!["auto".to_string()]
}

/// Overwrite `dst` with `src` unless `src` is `Auto` and `dst` is set —
/// preserves a "stronger" tier coming from the CLI override.
#[cfg(feature = "ocr")]
fn overwrite_tier(dst: &mut Tier, src: &mut Tier) {
    if !matches!(*src, Tier::Auto) {
        *dst = *src;
    }
}

/// Replace the language list when the override is non-empty; otherwise
/// keep the existing list. Mirrors how single-value `Option` merges work.
#[cfg(feature = "ocr")]
fn merge_languages(dst: &mut Vec<String>, src: &mut Vec<String>) {
    if !src.is_empty() {
        *dst = std::mem::take(src);
    }
}

#[cfg(feature = "ocr")]
impl OcrConfig {
    /// Returns `true` when OCR is enabled. Defaults to **true** when the
    /// field is missing from both the config file and the CLI — matching
    /// the product decision "OCR on by default".
    pub fn is_enabled(&self) -> bool {
        self.enable.unwrap_or(true)
    }

    /// Parses the configured language codes into [`Language`] enum values.
    pub fn languages(&self) -> Vec<Language> {
        sss_ocr::resolve_language(&self.language)
    }

    /// Effective tier after resolving `Auto` against the host hardware.
    pub fn effective_tier(&self) -> Tier {
        self.tier.resolve()
    }

    /// Selected ORT execution provider for OCR inference.
    pub fn gpu(&self) -> GpuMode {
        self.gpu
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

impl HasImports for ClapConfig {
    fn take_imports(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.root.imports)
    }
}

/// Fully-resolved CLI + config bundle returned by [`get_config`].
pub struct ResolvedConfig {
    pub cli: CliConfig,
    pub lib: sss_lib::GenerationSettings,
    pub ui: UiConfig,
    #[cfg(feature = "ocr")]
    pub ocr: OcrConfig,
}

pub fn get_config() -> Result<ResolvedConfig, ConfigurationError> {
    let mut args = ClapConfig::parse();

    let config_path = if let Some(path) = args.root.config.as_ref() {
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

    let loaded = load_with_imports(&config_path, &|s| toml::from_str::<ClapConfig>(s))
        .map_err(|e| match e {
            LoadError::Io(e) => ConfigurationError::Io(e),
            LoadError::Parse(e) => ConfigurationError::Deserialization(e),
        })?;

    let merged = if let Some(mut config) = loaded {
        tracing::debug!("Merging from config file");
        config.merge(&mut args);
        config
    } else {
        args
    };

    Ok(ResolvedConfig {
        cli: merged.cli.unwrap_or_default(),
        lib: merged.lib_config.into(),
        ui: merged.capture_ui.unwrap_or_default(),
        #[cfg(feature = "ocr")]
        ocr: merged.ocr.unwrap_or_default(),
    })
}
