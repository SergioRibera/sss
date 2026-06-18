//! Wires the `sss_capture_ui` interactive overlay into the CLI.
//!
//! The selector returns an already-decorated `RgbaImage` (brush strokes,
//! arrows, blur regions, etc. baked in) plus a `PostAction` describing
//! whether the user pressed Copy / Save. The CLI then runs that image
//! through `sss_lib::generate_image` to apply the rounded corners, shadow
//! and author footer that make `sss` recognisable.

use std::path::PathBuf;

use color_eyre::eyre::{eyre, Report};
use sss_capture_ui::{
    sss_capture::{BackendKind, CaptureOptions, Capturer},
    CaptureTrigger, OcrPipeline, Outcome, PostAction, SelectorBuilder, SelectorMode, TextClipboard,
    ToolKind, UiConfig,
};
use sss_lib::image::RgbaImage;
use sss_lib::GenerationSettings;
use std::sync::Arc;

use crate::config::CliConfig;
use crate::persist;

/// What the interactive selector produced. Consumed by `main`.
pub struct PreRendered {
    pub image: RgbaImage,
    pub action: PostAction,
    /// Default save path the CLI computed; the GUI's hint takes precedence.
    pub default_output: Option<PathBuf>,
}

/// Run the interactive selector. Returns:
///
/// * `Ok(Some(pre))` — the user committed (Enter / Save / Copy / Confirm).
/// * `Ok(None)` — the user cancelled (Esc / Cancel button). Cancellation
///   is not an error condition: scripts call us as `sss --area` and
///   need a non-zero exit *without* color_eyre splashing a backtrace.
///   Main turns the `None` into `std::process::exit(1)`.
/// * `Err(_)` — something actually broke (capturer init, etc.).
pub fn run(
    config: &CliConfig,
    g: &GenerationSettings,
    ui: &UiConfig,
    mode: SelectorMode,
    ocr_pipeline: Option<OcrPipeline>,
) -> Result<Option<PreRendered>, Report> {
    let default_output = if g.output.trim().is_empty() || g.output == "out.png" {
        Some(default_screenshot_path())
    } else {
        Some(PathBuf::from(&g.output))
    };

    let toolbar = !config.no_toolbar;
    let backend_kind = config
        .capture_backend
        .map(|c| c.to_kind())
        .unwrap_or(BackendKind::Auto);

    let capturer = Capturer::builder()
        .backend(backend_kind)
        .show_cursor(config.show_cursor)
        .build()
        .map_err(|e| eyre!("capturer build: {e}"))?;
    let capturer = Arc::new(capturer);
    tracing::info!(
        "interactive selector using backend: {}",
        capturer.backend_name()
    );

    let mut ui_config = ui.clone();
    if !toolbar {
        ui_config.tools = ToolKind::default_list();
        ui_config.initial_tool = ToolKind::Brush;
    }
    // Seed the in-session Border toggle from the CLI-resolved border
    // setting so `--no-border` opens the overlay with the button already
    // off and the user can flip it back on for just this capture.
    ui_config.border_enabled = g.border;

    let mut builder = SelectorBuilder::default()
        .mode(mode)
        .with_toolbar(toolbar)
        .ui(ui_config)
        .capture_trigger(CaptureTrigger::Eager)
        .capturer(capturer)
        .capture_options(CaptureOptions {
            show_cursor: config.show_cursor,
            ..Default::default()
        })
        .show_copy(!g.copy)
        .show_save(g.output.trim().is_empty() || g.output == "out.png");
    if let Some(path) = default_output.clone() {
        builder = builder.save_path_hint(path);
    }
    if config.remember_last_selection {
        if let Some(rect) = persist::load_last_area() {
            builder = builder.initial_area(rect);
        }
    }
    if let Some(pipeline) = ocr_pipeline {
        builder = builder.ocr_pipeline(pipeline);
        // Pair the OCR pipeline with the inline text-copy hook so Ctrl+C
        // (or the Copy toolbar icon) on an active OCR selection writes
        // the joined text to the system clipboard instead of confirming
        // an image copy + closing the overlay.
        let clip: TextClipboard = Arc::new(|text: &str| {
            sss_lib::copy_text_to_clipboard(text).map_err(|e| e.to_string())
        });
        builder = builder.text_clipboard(clip);
    }

    let selection = builder
        .build()
        .map_err(|e| eyre!("selector build: {e}"))?
        .run()
        .map_err(|e| eyre!("selector run: {e}"))?;

    let (image, last_region) = match selection.outcome {
        Outcome::Region {
            rect,
            image: Some(img),
        } => (img.into_rgba(), Some(rect)),
        Outcome::Monitor {
            image: Some(img), ..
        }
        | Outcome::Window {
            image: Some(img), ..
        } => (img.into_rgba(), None),
        // Cancellation is the user's explicit choice (Esc / Cancel
        // button). Surface it as `Ok(None)` so the CLI can exit with a
        // non-zero status without color_eyre's full error chrome.
        Outcome::Cancelled => return Ok(None),
        _ => return Err(eyre!("selector returned without an image")),
    };

    if config.remember_last_selection {
        if let Some(rect) = last_region {
            persist::save_last_area(rect);
        }
    }

    Ok(Some(PreRendered {
        image,
        action: selection.action,
        default_output,
    }))
}

fn default_screenshot_path() -> PathBuf {
    // Mirrors `grim` / GNOME Screenshot defaults: ~/Pictures with a timestamp.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let base = directories::UserDirs::new()
        .and_then(|d| d.picture_dir().map(|p| p.to_path_buf()))
        .unwrap_or_else(std::env::temp_dir);
    base.join(format!("sss-{stamp}.png"))
}
