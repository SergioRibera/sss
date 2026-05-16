//! Public entry point for the interactive overlay.

use std::path::PathBuf;
use std::sync::Arc;

use sss_capture::{CaptureError, CaptureOptions, Capturer, Image, MonitorId, Rect, WindowId};
use thiserror::Error;

use crate::canvas::Canvas;
use crate::config::UiConfig;
use crate::mode::SelectorMode;
use crate::tool::ToolPalette;
use crate::trigger::{CaptureTrigger, KeyBind};

/// What the overlay produced.
#[derive(Clone, Debug)]
pub enum Outcome {
    Region {
        rect: Rect,
        image: Option<Image>,
    },
    Monitor {
        monitor: MonitorId,
        rect: Rect,
        image: Option<Image>,
    },
    Window {
        window: WindowId,
        rect: Rect,
        image: Option<Image>,
    },
    Cancelled,
}

impl Outcome {
    pub fn image(&self) -> Option<&Image> {
        match self {
            Outcome::Region { image, .. }
            | Outcome::Monitor { image, .. }
            | Outcome::Window { image, .. } => image.as_ref(),
            Outcome::Cancelled => None,
        }
    }

    /// Take the captured image out of the outcome.
    pub fn take_image(self) -> Option<Image> {
        match self {
            Outcome::Region { image, .. }
            | Outcome::Monitor { image, .. }
            | Outcome::Window { image, .. } => image,
            Outcome::Cancelled => None,
        }
    }

    /// Rectangle of the selection in virtual-desktop coordinates.
    pub fn rect(&self) -> Option<Rect> {
        match self {
            Outcome::Region { rect, .. }
            | Outcome::Monitor { rect, .. }
            | Outcome::Window { rect, .. } => Some(*rect),
            Outcome::Cancelled => None,
        }
    }
}

/// Action the user signalled before closing the overlay.
///
/// The host (sss CLI, an editor app, …) decides what to do with this — the
/// selector itself never writes files or touches the clipboard. The intent
/// is to let the GUI override / supply defaults that the caller may have
/// left blank in its own CLI flags.
#[derive(Clone, Debug, Default)]
pub struct PostAction {
    /// User asked to copy the result. Triggered by the toolbar's Copy
    /// button or by `copy_keybind` (default `Ctrl+C`).
    pub copy: bool,
    /// User asked to save. Triggered by the toolbar's Save button or by
    /// `save_keybind` (default `Ctrl+S`). The path is the one the host
    /// supplied through [`SelectorBuilder::save_path_hint`]; the selector
    /// only flips the `save` flag — picking a file path is the host's job.
    pub save: bool,
    /// Suggested save path inherited from the builder. The host may use it
    /// as a default for a file-chooser dialog or write directly to it.
    pub save_path_hint: Option<PathBuf>,
}

/// Aggregate result of a [`Selector::run`] call.
#[derive(Clone, Debug)]
pub struct Selection {
    pub outcome: Outcome,
    pub canvas: Canvas,
    pub action: PostAction,
}

/// Builder for [`Selector`].
#[derive(Debug)]
pub struct SelectorBuilder {
    mode: SelectorMode,
    toolbar: bool,
    ui: UiConfig,
    palette_override: Option<ToolPalette>,
    trigger: CaptureTrigger,
    capturer: Option<Arc<Capturer>>,
    capture_opts: CaptureOptions,
    confirm_with_enter: bool,
    show_copy: bool,
    show_save: bool,
    save_path_hint: Option<PathBuf>,
}

impl Default for SelectorBuilder {
    fn default() -> Self {
        Self {
            mode: SelectorMode::default(),
            toolbar: false,
            ui: UiConfig::default(),
            palette_override: None,
            trigger: CaptureTrigger::default(),
            capturer: None,
            capture_opts: CaptureOptions::default(),
            confirm_with_enter: true,
            show_copy: true,
            show_save: true,
            save_path_hint: None,
        }
    }
}

impl SelectorBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mode(mut self, mode: SelectorMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_toolbar(mut self, on: bool) -> Self {
        self.toolbar = on;
        self
    }

    pub fn ui(mut self, ui: UiConfig) -> Self {
        self.ui = ui;
        self
    }

    pub fn with_ui<F: FnOnce(&mut UiConfig)>(mut self, f: F) -> Self {
        f(&mut self.ui);
        self
    }

    /// Override the derived tool palette with a hand-built one.
    pub fn palette(mut self, palette: ToolPalette) -> Self {
        self.ui.palette = palette.color_palette.clone();
        self.palette_override = Some(palette);
        self
    }

    pub fn capture_trigger(mut self, t: CaptureTrigger) -> Self {
        self.trigger = t;
        self
    }

    /// Use an existing capturer instance. When `None` the selector builds one
    /// at run-time with [`Capturer::new`].
    pub fn capturer(mut self, c: Arc<Capturer>) -> Self {
        self.capturer = Some(c);
        self
    }

    pub fn capture_options(mut self, opts: CaptureOptions) -> Self {
        self.capture_opts = opts;
        self
    }

    /// When `true` (default) Enter confirms; when `false` the selector only
    /// closes through the toolbar's `Capture` button.
    pub fn enter_to_confirm(mut self, on: bool) -> Self {
        self.confirm_with_enter = on;
        self
    }

    /// Show a Copy button in the toolbar. The button (or its keybind,
    /// default `Ctrl+C`) flips [`PostAction::copy`] before the overlay
    /// closes. Default: `true`.
    pub fn show_copy(mut self, on: bool) -> Self {
        self.show_copy = on;
        self
    }

    /// Show a Save button in the toolbar. The button (or its keybind,
    /// default `Ctrl+S`) flips [`PostAction::save`] before the overlay
    /// closes. Default: `true`.
    pub fn show_save(mut self, on: bool) -> Self {
        self.show_save = on;
        self
    }

    /// Default save path emitted on `PostAction::save_path_hint`. Hosts
    /// (sss CLI) read it when no `--output` flag was supplied.
    pub fn save_path_hint(mut self, path: impl Into<PathBuf>) -> Self {
        self.save_path_hint = Some(path.into());
        self
    }

    pub fn build(self) -> Result<Selector, SelectorError> {
        let capturer = match self.capturer {
            Some(c) => c,
            None => Arc::new(Capturer::new().map_err(SelectorError::Capture)?),
        };
        let palette = self
            .palette_override
            .clone()
            .unwrap_or_else(|| self.ui.build_tool_palette());
        Ok(Selector {
            config: Config {
                mode: self.mode,
                toolbar: self.toolbar,
                palette,
                ui: self.ui,
                trigger: self.trigger,
                capture_opts: self.capture_opts,
                confirm_with_enter: self.confirm_with_enter,
                show_copy: self.show_copy,
                show_save: self.show_save,
                save_path_hint: self.save_path_hint,
            },
            capturer,
        })
    }
}

/// Configured selector.
#[derive(Debug)]
pub struct Selector {
    pub(crate) config: Config,
    pub(crate) capturer: Arc<Capturer>,
}

#[derive(Clone, Debug)]
pub(crate) struct Config {
    pub mode: SelectorMode,
    pub toolbar: bool,
    pub palette: ToolPalette,
    pub ui: UiConfig,
    pub trigger: CaptureTrigger,
    pub capture_opts: CaptureOptions,
    pub confirm_with_enter: bool,
    pub show_copy: bool,
    pub show_save: bool,
    pub save_path_hint: Option<PathBuf>,
}

impl Selector {
    pub fn builder() -> SelectorBuilder {
        SelectorBuilder::new()
    }

    /// Run the overlay. Blocks the current thread until the user confirms,
    /// cancels (`Escape`), or closes the overlay window.
    pub fn run(self) -> Result<Selection, SelectorError> {
        crate::platform::run(self)
    }
}

/// Error returned by the selector pipeline.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SelectorError {
    #[error(transparent)]
    Capture(CaptureError),
    #[error("UI backend error: {0}")]
    Backend(String),
    #[error("the editor feature is disabled but a toolbar was requested")]
    EditorNotEnabled,
}

impl From<CaptureError> for SelectorError {
    fn from(e: CaptureError) -> Self {
        SelectorError::Capture(e)
    }
}

#[allow(dead_code)]
pub(crate) fn _silence_keybind(_: KeyBind) {}
