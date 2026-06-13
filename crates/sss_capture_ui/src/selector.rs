//! Public entry point for the interactive overlay.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use image::RgbaImage;
use sss_capture::{CaptureError, CaptureOptions, Capturer, Image, MonitorId, Rect, WindowId};
use sss_core::ocr::TextBox;
use thiserror::Error;

use crate::canvas::Canvas;
use crate::config::UiConfig;
use crate::mode::SelectorMode;
use crate::tool::ToolPalette;
use crate::trigger::{CaptureTrigger, KeyBind};

/// Hook the CLI plugs in to run OCR over the eager-captured screenshot.
///
/// The selector calls the closure once, immediately after the eager
/// capture succeeds, and stores the returned `Receiver`. Each redraw it
/// `try_recv`s; the first message becomes [`Canvas::set_text_boxes`].
///
/// Returning `Receiver<Vec<TextBox>>` instead of a future keeps this crate
/// runtime-agnostic — the implementation just spawns a `std::thread`.
pub type OcrPipeline =
    Arc<dyn Fn(RgbaImage) -> Receiver<Vec<TextBox>> + Send + Sync>;

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

    pub fn take_image(self) -> Option<Image> {
        match self {
            Outcome::Region { image, .. }
            | Outcome::Monitor { image, .. }
            | Outcome::Window { image, .. } => image,
            Outcome::Cancelled => None,
        }
    }

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
#[derive(Clone, Debug, Default)]
pub struct PostAction {
    pub copy: bool,
    pub save: bool,
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
    initial_area: Option<Rect>,
    ocr_pipeline: Option<OcrPipeline>,
}

impl std::fmt::Debug for SelectorBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectorBuilder")
            .field("mode", &self.mode)
            .field("toolbar", &self.toolbar)
            .field("ui", &self.ui)
            .field("palette_override", &self.palette_override)
            .field("trigger", &self.trigger)
            .field("capturer", &self.capturer)
            .field("capture_opts", &self.capture_opts)
            .field("confirm_with_enter", &self.confirm_with_enter)
            .field("show_copy", &self.show_copy)
            .field("show_save", &self.show_save)
            .field("save_path_hint", &self.save_path_hint)
            .field("initial_area", &self.initial_area)
            .field("ocr_pipeline", &self.ocr_pipeline.as_ref().map(|_| "<fn>"))
            .finish()
    }
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
            initial_area: None,
            ocr_pipeline: None,
        }
    }
}

impl SelectorBuilder {
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

    pub fn capturer(mut self, c: Arc<Capturer>) -> Self {
        self.capturer = Some(c);
        self
    }

    pub fn capture_options(mut self, opts: CaptureOptions) -> Self {
        self.capture_opts = opts;
        self
    }

    pub fn enter_to_confirm(mut self, on: bool) -> Self {
        self.confirm_with_enter = on;
        self
    }

    pub fn show_copy(mut self, on: bool) -> Self {
        self.show_copy = on;
        self
    }

    pub fn show_save(mut self, on: bool) -> Self {
        self.show_save = on;
        self
    }

    pub fn save_path_hint(mut self, path: impl Into<PathBuf>) -> Self {
        self.save_path_hint = Some(path.into());
        self
    }

    /// Pre-seed the area selector with a rectangle. The overlay opens with
    /// this region already drawn, ready to be confirmed or adjusted. Only
    /// honoured in `Area` / `AnyOf` modes.
    pub fn initial_area(mut self, rect: Rect) -> Self {
        self.initial_area = Some(rect);
        self
    }

    /// Plug an OCR pipeline in. When set, the eager-captured frame is
    /// pushed into the closure as soon as the overlay opens; results
    /// flow back through the returned `Receiver` and end up in the
    /// canvas via [`Canvas::set_text_boxes`].
    pub fn ocr_pipeline(mut self, pipeline: OcrPipeline) -> Self {
        self.ocr_pipeline = Some(pipeline);
        self
    }

    pub fn build(self) -> Result<Selector, SelectorError> {
        let capturer = match self.capturer {
            Some(c) => c,
            None => Arc::new(
                Capturer::builder()
                    .build()
                    .map_err(SelectorError::Capture)?,
            ),
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
                initial_area: self.initial_area,
                ocr_pipeline: self.ocr_pipeline,
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

#[derive(Clone)]
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
    pub initial_area: Option<Rect>,
    pub ocr_pipeline: Option<OcrPipeline>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("mode", &self.mode)
            .field("toolbar", &self.toolbar)
            .field("palette", &self.palette)
            .field("ui", &self.ui)
            .field("trigger", &self.trigger)
            .field("capture_opts", &self.capture_opts)
            .field("confirm_with_enter", &self.confirm_with_enter)
            .field("show_copy", &self.show_copy)
            .field("show_save", &self.show_save)
            .field("save_path_hint", &self.save_path_hint)
            .field("initial_area", &self.initial_area)
            .field("ocr_pipeline", &self.ocr_pipeline.as_ref().map(|_| "<fn>"))
            .finish()
    }
}

impl Selector {
    pub fn builder() -> SelectorBuilder {
        SelectorBuilder::default()
    }

    /// Run the overlay, blocking until the user confirms or cancels.
    pub fn run(self) -> Result<Selection, SelectorError> {
        crate::platform::run(self)
    }
}

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
