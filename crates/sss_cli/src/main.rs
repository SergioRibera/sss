use std::sync::{Arc, Mutex};

use color_eyre::eyre::Report;
use config::{get_config, OcrConfig};
use img::Screenshot;
use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};
use sss_capture_ui::{OcrPipeline, SelectorMode};
use sss_lib::generate_image;
use sss_lib::image::RgbaImage;
use sss_ocr::{GpuMode, Language, OcrEngine, PrewarmHandle, PrewarmStatus, PrewarmWaiter};
use tracing_subscriber::EnvFilter;

mod config;
mod error;
mod img;
mod interactive;
mod persist;
mod shot;

#[derive(Clone, Copy, Debug, Default)]
pub struct Area {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

fn main() -> Result<(), Report> {
    // Default: warn-and-above for our code, but silence winit-wayland's
    // expected layer-shell complaints (xdg_toplevel / min-max size unsupported
    // — those are inherent to the protocol we deliberately use).
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("warn,winit_wayland=error,sctk=error,wayland_client=error")
    });
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .with_timer(tracing_subscriber::fmt::time::Uptime::default())
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    // install color eyre
    color_eyre::config::HookBuilder::default()
        .issue_url(concat!(env!("CARGO_PKG_REPOSITORY"), "/issues/new"))
        .add_issue_metadata("version", env!("CARGO_PKG_VERSION"))
        .issue_filter(|kind| match kind {
            color_eyre::ErrorKind::NonRecoverable(_) => false,
            color_eyre::ErrorKind::Recoverable(_) => true,
        })
        .install()?;

    let config::ResolvedConfig {
        cli: config,
        lib: mut g_config,
        ui: ui_config,
        ocr: ocr_config,
    } = get_config()?;
    tracing::info!(
        enabled = ocr_config.is_enabled(),
        tier = ?ocr_config.effective_tier(),
        languages = ?ocr_config.languages(),
        formula = ocr_config.formula,
        gpu = ?ocr_config.gpu(),
        "OCR configuration"
    );
    let prewarm = start_prewarm(&ocr_config);
    let ocr_pipeline = build_ocr_pipeline(&ocr_config, prewarm.as_ref().map(|h| h.waiter()));
    if config.verbose {
        // Re-init at info level by overriding the existing filter. We do
        // that lazily here so the verbose flag is read after parsing.
        std::env::set_var("RUST_LOG", "info");
    }
    tracing::trace!("Configs loaded");

    // The CLI is interactive whenever the user did NOT supply a complete
    // targeting flag, OR when `--interactive` is forced.
    //   * `--area "x,y WxH"`              → direct
    //   * `--area`                        → selector in Area mode
    //   * `--screen-id <v>`               → direct
    //   * `--screen-id`                   → selector in Monitor mode
    //   * `--window <v>`                  → direct
    //   * `--window`                      → selector in Window mode
    //   * `--screen --current`            → direct (monitor under cursor)
    //   * `--screen` alone                → selector in Monitor mode
    //   * `--current` alone               → direct (monitor under cursor)
    //   * (none of the above)             → selector in AnyOf mode
    let direct = config.direct_target();
    let want_interactive = config.interactive || direct.is_none();

    if want_interactive {
        let mode = pick_initial_mode(&config);
        // `interactive::run` returns `Ok(None)` for user cancellation
        // (Esc / Cancel button). That's not a real error — it's just
        // "user changed their mind". We exit 1 directly so scripts can
        // detect the cancel, but without color_eyre's big error chrome
        // which would otherwise present cancellation as a crash.
        let pre = match interactive::run(
            &config,
            &g_config,
            &ui_config,
            mode,
            ocr_pipeline.clone(),
        )? {
            Some(pre) => pre,
            None => {
                // User pressed Esc / Cancel. Honour the "the download
                // thread keeps running until the first download is done"
                // requirement: block on the prewarm worker before
                // exiting with the cancellation status code.
                finish_prewarm(prewarm);
                std::process::exit(1);
            }
        };
        // The GUI may have flipped Copy / Save intent. Honour them only when
        // the CLI itself didn't already specify them.
        if pre.action.copy && !g_config.copy {
            g_config.copy = true;
        }
        if pre.action.save && (g_config.output.trim().is_empty() || g_config.output == "out.png") {
            if let Some(path) = pre
                .action
                .save_path_hint
                .clone()
                .or_else(|| pre.default_output.clone())
            {
                g_config.output = path.to_string_lossy().into_owned();
            }
        }
        let result = generate_image(g_config, Screenshot::pre_rendered(pre.image));
        finish_prewarm(prewarm);
        return Ok(result?);
    }

    let result = generate_image(
        g_config,
        Screenshot::from_target(direct.unwrap(), config.show_cursor),
    );
    finish_prewarm(prewarm);
    Ok(result?)
}

/// Spawns the OCR model-prewarm worker when OCR is enabled, otherwise
/// returns `None`.
///
/// Honours `[ocr].models-dir` from the config file: when set, the worker
/// downloads into that directory instead of the default XDG cache. The
/// path is committed to `OAR_HOME` for the lifetime of the process.
fn start_prewarm(ocr: &OcrConfig) -> Option<PrewarmHandle> {
    if !ocr.is_enabled() {
        return None;
    }
    sss_ocr::install_models_dir_with(ocr.models_dir.clone());
    Some(sss_ocr::spawn_prewarm(
        ocr.effective_tier(),
        ocr.languages(),
        ocr.formula,
    ))
}

/// Builds the OCR submission closure passed to the interactive selector.
///
/// Each call to the returned closure spawns a one-shot worker that:
///   1. blocks on `waiter` (so OCR can't run until models are on disk);
///   2. builds an [`OcrEngine`] for the first configured language;
///   3. runs recognition on the supplied RGBA frame;
///   4. emits the resulting `Vec<TextBox>` through an `mpsc::channel`.
///
/// Returning `None` keeps the selector's OCR rx empty (and the canvas
/// reports zero detections), which is the disabled-OCR path.
fn build_ocr_pipeline(
    ocr: &OcrConfig,
    waiter: Option<PrewarmWaiter>,
) -> Option<OcrPipeline> {
    if !ocr.is_enabled() {
        return None;
    }
    let tier = ocr.effective_tier();
    let languages = ocr.languages();
    let formula = ocr.formula;
    let gpu = ocr.gpu();
    // The engine load is the slow part of an OCR run (model parse, ORT
    // session init). Cache it across dispatches so re-running OCR after a
    // region change costs only the forward pass, not another cold build.
    let engine_cache: Arc<Mutex<Option<Arc<OcrEngine>>>> = Arc::new(Mutex::new(None));
    let pipeline: OcrPipeline = Arc::new(move |image: RgbaImage| {
        let (tx, rx) = std::sync::mpsc::channel();
        let waiter = waiter.clone();
        let languages = languages.clone();
        let engine_cache = engine_cache.clone();
        std::thread::Builder::new()
            .name("sss-ocr-worker".into())
            .spawn(move || {
                if let Some(w) = waiter {
                    if matches!(w.block_until_done(), PrewarmStatus::Failed) {
                        tracing::warn!("OCR worker giving up: prewarm failed");
                        return;
                    }
                }
                let engine = match get_or_build_engine(&engine_cache, tier, &languages, formula, gpu) {
                    Some(e) => e,
                    None => return,
                };
                match engine.run(&image) {
                    Ok(boxes) => {
                        let _ = tx.send(boxes);
                    }
                    Err(err) => tracing::warn!(%err, "OCR inference failed"),
                }
            })
            .expect("failed to spawn sss-ocr-worker thread");
        rx
    });
    Some(pipeline)
}

/// Lazily build (and memoise) a single [`OcrEngine`] for the chosen
/// `(tier, language, formula)`. The first caller pays the cold-start cost;
/// every subsequent caller — e.g. a re-OCR triggered by the user resizing
/// the selection region — just clones the `Arc`.
fn get_or_build_engine(
    cache: &Arc<Mutex<Option<Arc<OcrEngine>>>>,
    tier: sss_ocr::Tier,
    languages: &[Language],
    formula: bool,
    gpu: GpuMode,
) -> Option<Arc<OcrEngine>> {
    {
        let guard = cache.lock().unwrap();
        if let Some(e) = guard.as_ref() {
            return Some(e.clone());
        }
    }
    let language = languages.first().copied().unwrap_or(Language::Auto);
    let engine = match OcrEngine::new(tier, language, formula, gpu) {
        Ok(e) => Arc::new(e),
        Err(err) => {
            tracing::warn!(%err, "OCR engine build failed");
            return None;
        }
    };
    let mut guard = cache.lock().unwrap();
    if let Some(e) = guard.as_ref() {
        return Some(e.clone());
    }
    *guard = Some(engine.clone());
    Some(engine)
}

/// Blocks until the prewarm worker finishes downloading every model.
///
/// We deliberately block at end-of-run rather than at start: a screenshot
/// session does not need the models, only the optional OCR overlay does.
/// Joining here guarantees that closing sss never leaves a half-downloaded
/// file behind, which is what the user asked for ("si se cierra sss el
/// hilo de descarga siga hasta terminar y recien se cierra el proceso de
/// sss completo").
fn finish_prewarm(prewarm: Option<PrewarmHandle>) {
    let Some(handle) = prewarm else {
        return;
    };
    if handle.is_finished() {
        let _ = handle.wait();
        return;
    }
    tracing::info!("waiting for OCR model download to finish before exiting");
    if let Err(err) = handle.wait() {
        tracing::warn!(%err, "OCR model prewarm failed; OCR will be unavailable next run");
    }
}

/// Decide which mode the selector should open in based on which targeting
/// flag the user supplied without a value.
fn pick_initial_mode(config: &config::CliConfig) -> SelectorMode {
    use config::{AreaSpec, ScreenSpec, WindowSpec};
    if matches!(config.area, Some(AreaSpec::Interactive)) {
        SelectorMode::Area
    } else if matches!(config.window, Some(WindowSpec::Interactive)) {
        SelectorMode::Window
    } else if matches!(config.screen_id, Some(ScreenSpec::Interactive))
        || (config.screen && !config.current)
    {
        SelectorMode::Monitor
    } else {
        SelectorMode::AnyOf
    }
}

fn str_to_area(s: &str) -> Result<Area, String> {
    let err = "The format of area is wrong (x,y WxH)".to_string();
    let (pos, size) = s.split_once(' ').ok_or(err.clone())?;
    let (x, y) = pos.split_once(',').ok_or(err.clone()).map(|(x, y)| {
        (
            x.parse::<i32>().map_err(|e| e.to_string()),
            y.parse::<i32>().map_err(|e| e.to_string()),
        )
    })?;
    let (w, h) = size.split_once('x').ok_or(err.clone()).map(|(w, h)| {
        (
            w.parse::<u32>().map_err(|e| e.to_string()),
            h.parse::<u32>().map_err(|e| e.to_string()),
        )
    })?;

    Ok(Area {
        x: x?,
        y: y?,
        width: w?,
        height: h?,
    })
}

impl<'de> Deserialize<'de> for Area {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        str_to_area(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl Serialize for Area {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let Area {
            x,
            y,
            width,
            height,
        } = self;
        String::serialize(&format!("{x},{y} {width}x{height}"), serializer)
    }
}
