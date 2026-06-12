//! Background download / model-prewarm worker.
//!
//! oar-ocr's `auto-download` feature pulls bare model names from ModelScope
//! the first time a builder hits them. We piggy-back on that: in a worker
//! thread we construct the pipeline (which triggers the download), then
//! drop it. From then on the on-disk cache is warm and [`OcrEngine`] can
//! load instantly from any thread.
//!
//! The handle returned by [`spawn_prewarm`] does **not** detach: callers are
//! expected to `join` it (or wait on its status channel) before exiting the
//! process, so cancelling a screenshot never leaves a half-downloaded model
//! on disk.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::thread::{self, JoinHandle};

use crate::engine::OcrEngine;
use crate::error::OcrError;
use crate::hardware::Tier;
use crate::registry::{Language, union_files};

/// Coarse state machine for the prewarm worker.
///
/// Encoded as a `u8` so it lives in an `AtomicU8` and survives the worker's
/// own `JoinHandle` going out of scope on the producer side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PrewarmStatus {
    /// Models were already present on disk; the worker exits immediately.
    Ready = 0,
    /// Worker is still downloading at least one model.
    InProgress = 1,
    /// Worker finished and every model is on disk.
    Done = 2,
    /// Worker failed; the engine cannot be constructed.
    Failed = 3,
}

impl PrewarmStatus {
    fn from_u8(n: u8) -> Self {
        match n {
            0 => Self::Ready,
            1 => Self::InProgress,
            2 => Self::Done,
            _ => Self::Failed,
        }
    }
}

/// Handle to a running prewarm worker.
///
/// Drop semantics: dropping the handle without calling [`Self::wait`] will
/// detach the thread; the OS will still let it finish writing its file. The
/// callers in `sss_cli` join on exit so this only matters for tests.
pub struct PrewarmHandle {
    join: Option<JoinHandle<Result<(), OcrError>>>,
    status: Arc<AtomicU8>,
}

impl PrewarmHandle {
    /// Returns the current state without blocking.
    pub fn status(&self) -> PrewarmStatus {
        PrewarmStatus::from_u8(self.status.load(Ordering::Acquire))
    }

    /// Blocks until the worker finishes, returning any error that occurred.
    pub fn wait(mut self) -> Result<(), OcrError> {
        if let Some(handle) = self.join.take() {
            match handle.join() {
                Ok(r) => r,
                Err(_) => Err(OcrError::Oar("prewarm worker panicked".into())),
            }
        } else {
            Ok(())
        }
    }

    /// Returns true once the worker has reached a terminal state
    /// (`Ready`, `Done`, or `Failed`).
    pub fn is_finished(&self) -> bool {
        !matches!(self.status(), PrewarmStatus::InProgress)
    }
}

/// Spawns a worker thread that constructs each `(tier, language)` pipeline
/// once so oar-ocr's auto-download fetches every required file.
///
/// `formula` opts into downloading the formula model too. `languages` may
/// contain multiple entries; every recogniser gets pre-fetched so the user
/// can switch language later without re-downloading.
pub fn spawn_prewarm(
    tier: Tier,
    languages: Vec<Language>,
    formula: bool,
) -> PrewarmHandle {
    crate::install_models_dir();
    let status = Arc::new(AtomicU8::new(PrewarmStatus::InProgress as u8));
    let status_thread = Arc::clone(&status);

    let join = thread::Builder::new()
        .name("sss-ocr-prewarm".into())
        .spawn(move || -> Result<(), OcrError> {
            let tier = tier.resolve();
            let pending = union_files(tier, &languages, formula);
            tracing::info!(?tier, files = ?pending, "prewarm plan");
            for lang in &languages {
                tracing::info!(?tier, ?lang, formula, "prewarming OCR models");
                // Constructing the engine runs the full ONNX session
                // init too. That's slower than strictly needed for pure
                // download, but it also catches genuinely broken model
                // files before the user clicks Capture.
                let _engine = OcrEngine::new(tier, *lang, formula)?;
            }
            status_thread.store(PrewarmStatus::Done as u8, Ordering::Release);
            Ok(())
        })
        .expect("failed to spawn sss-ocr-prewarm thread");

    PrewarmHandle {
        join: Some(join),
        status,
    }
}
