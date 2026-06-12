//! OCR engine for sss.
//!
//! Wraps `oar-ocr` with:
//! - tiered defaults sized to host hardware,
//! - language → model resolution against the PaddleOCR PP-OCRv5 family,
//! - a non-blocking pre-warm worker that forces model download on first run,
//! - a single sync [`OcrEngine`] handle the UI can call from a worker thread.

mod engine;
mod error;
mod hardware;
mod prewarm;
mod registry;
mod types;

pub use engine::OcrEngine;
pub use error::OcrError;
pub use hardware::{Tier, resolve_tier};
pub use prewarm::{PrewarmHandle, PrewarmStatus, spawn_prewarm};
pub use registry::{Language, ModelSet, resolve_language, resolve_models, union_files};
pub use types::{TextBox, TextPoint};

use std::path::{Path, PathBuf};

/// Returns the directory where OCR models are cached.
///
/// Honours `OAR_HOME` (oar-ocr's own env var) when set, otherwise
/// `$XDG_DATA_HOME/sss/models` (or platform equivalent via [`directories`]).
pub fn models_dir() -> PathBuf {
    if let Ok(env) = std::env::var("OAR_HOME") {
        return PathBuf::from(env);
    }
    directories::BaseDirs::new()
        .map(|d| d.data_dir().join("sss").join("models"))
        .unwrap_or_else(|| PathBuf::from(".sss-models"))
}

/// Sets `OAR_HOME` to [`models_dir`] for the current process so oar-ocr's
/// auto-download writes into our XDG-friendly location instead of `~/.oar`.
///
/// Safe to call more than once; only the first call wins.
pub fn install_models_dir() {
    install_models_dir_with(None);
}

/// Like [`install_models_dir`] but lets the caller override the cache root.
///
/// When `custom` is `Some`, that path becomes `OAR_HOME`. When `None`,
/// behaves identically to [`install_models_dir`]. Only the **first** call
/// to either function wins — subsequent calls are no-ops, so the caller in
/// `sss_cli` must invoke this before any [`OcrEngine`] is built.
pub fn install_models_dir_with(custom: Option<PathBuf>) {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = custom.unwrap_or_else(models_dir);
        // Best-effort: oar-ocr will create the dir itself on first download,
        // but creating it eagerly lets `dir.exists()` checks elsewhere succeed.
        let _ = std::fs::create_dir_all(&dir);
        // SAFETY: set_var is unsafe in 2024 edition because of multi-threaded
        // races; install_models_dir is documented "call early, single-threaded"
        // and the Once gate guarantees a single write.
        unsafe { std::env::set_var("OAR_HOME", dir.as_os_str()) };
    });
}

/// Returns true if every model file in `set` is already present under
/// [`models_dir`].
pub fn models_present(set: &ModelSet) -> bool {
    let dir = models_dir();
    set.files().all(|name| dir.join(name).exists())
}

/// Returns the path under [`models_dir`] for a registered file name.
pub fn model_path(name: &str) -> PathBuf {
    models_dir().join(name)
}

/// Returns true if the path resolves to an existing file under [`models_dir`].
pub fn model_present(name: impl AsRef<Path>) -> bool {
    model_path(&name.as_ref().to_string_lossy()).exists()
}
