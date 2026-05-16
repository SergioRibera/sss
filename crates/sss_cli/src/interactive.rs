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
    CaptureTrigger, Outcome, PostAction, SelectorBuilder, SelectorMode, ToolKind, UiConfig,
};
use sss_lib::image::RgbaImage;
use sss_lib::GenerationSettings;
use std::sync::Arc;

use crate::config::CliConfig;

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

    let mut builder = SelectorBuilder::new()
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

    let selection = builder
        .build()
        .map_err(|e| eyre!("selector build: {e}"))?
        .run()
        .map_err(|e| eyre!("selector run: {e}"))?;

    let image = match selection.outcome {
        Outcome::Region {
            image: Some(img), ..
        }
        | Outcome::Monitor {
            image: Some(img), ..
        }
        | Outcome::Window {
            image: Some(img), ..
        } => img.into_rgba(),
        // Cancellation is the user's explicit choice (Esc / Cancel
        // button). Surface it as `Ok(None)` so the CLI can exit with a
        // non-zero status without color_eyre's full error chrome.
        Outcome::Cancelled => return Ok(None),
        _ => return Err(eyre!("selector returned without an image")),
    };

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
        .unwrap_or_else(|| std::env::temp_dir());
    base.join(format!("sss-{stamp}.png"))
}
