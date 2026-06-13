//! Background download / model-prewarm worker.
//!
//! oar-ocr's `auto-download` feature pulls bare model names from ModelScope
//! the first time a builder hits them. We piggy-back on that with a worker
//! thread that walks the union of required files for the configured
//! tier × languages × formula combo, downloads each one explicitly, and
//! prints a live progress bar to stderr so the user sees how much is left.
//!
//! The handle returned by [`spawn_prewarm`] does **not** detach: callers are
//! expected to `join` it (or wait on its status channel) before exiting the
//! process, so cancelling a screenshot never leaves a half-downloaded model
//! on disk.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use oar_ocr::download;

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

    /// Cheap clonable token that lets a background thread block on the
    /// prewarm result without owning the [`PrewarmHandle`] itself (the
    /// `JoinHandle` is consumed by [`Self::wait`] in `main`).
    pub fn waiter(&self) -> PrewarmWaiter {
        PrewarmWaiter {
            status: Arc::clone(&self.status),
        }
    }
}

/// Shared, lock-free view of a prewarm worker's status.
///
/// Clones over to any thread that wants to gate work on download completion
/// (e.g. the per-capture OCR submission worker that fires once the user
/// has confirmed a region).
#[derive(Debug, Clone)]
pub struct PrewarmWaiter {
    status: Arc<AtomicU8>,
}

impl PrewarmWaiter {
    pub fn status(&self) -> PrewarmStatus {
        PrewarmStatus::from_u8(self.status.load(Ordering::Acquire))
    }

    /// Spin-with-sleep until the worker leaves [`PrewarmStatus::InProgress`].
    ///
    /// Polls every 150 ms; OCR submission is already off the UI thread so
    /// the small latency is invisible to the user.
    pub fn block_until_done(&self) -> PrewarmStatus {
        loop {
            let s = self.status();
            if !matches!(s, PrewarmStatus::InProgress) {
                return s;
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
    }
}

/// Spawns a worker thread that downloads every model file required for
/// the requested tier × languages × formula combination, printing a live
/// progress bar to stderr so the user can see how much is left.
///
/// The thread skips files that are already cached at the correct size, so
/// subsequent launches are effectively free.
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
            let resolved_tier = tier.resolve();
            let plan = union_files(resolved_tier, &languages, formula);
            tracing::info!(?resolved_tier, files = ?plan, "prewarm plan");
            let dir = download::cache_dir();
            let pending: Vec<&'static str> =
                plan.iter().copied().filter(|name| !is_cached(&dir, name)).collect();
            if pending.is_empty() {
                eprintln!("[OCR] models already cached at {}", dir.display());
                status_thread.store(PrewarmStatus::Ready as u8, Ordering::Release);
                return Ok(());
            }
            let total_bytes: u64 = pending
                .iter()
                .filter_map(|n| download::find(n).map(|e| e.size))
                .sum();
            eprintln!(
                "[OCR] downloading {} model file(s), {} total → {}",
                pending.len(),
                human_bytes(total_bytes),
                dir.display()
            );
            for (i, file) in pending.iter().enumerate() {
                let size = download::find(file).map(|e| e.size).unwrap_or(0);
                eprintln!(
                    "[OCR] {}/{}  {}  ({})",
                    i + 1,
                    pending.len(),
                    file,
                    human_bytes(size)
                );
                let stop = Arc::new(AtomicBool::new(false));
                let stop_watch = Arc::clone(&stop);
                let dir_watch = dir.clone();
                let name_watch = file.to_string();
                let watcher = thread::Builder::new()
                    .name(format!("sss-ocr-progress-{file}"))
                    .spawn(move || progress_loop(&dir_watch, &name_watch, size, &stop_watch))
                    .expect("failed to spawn progress watcher");
                let result = download::fetch(file).map_err(OcrError::from);
                stop.store(true, Ordering::Release);
                let _ = watcher.join();
                if let Err(err) = result {
                    eprintln!("[OCR] failed to download {file}: {err}");
                    status_thread.store(PrewarmStatus::Failed as u8, Ordering::Release);
                    return Err(err);
                }
            }
            eprintln!("[OCR] all models ready");
            status_thread.store(PrewarmStatus::Done as u8, Ordering::Release);
            Ok(())
        })
        .expect("failed to spawn sss-ocr-prewarm thread");

    PrewarmHandle {
        join: Some(join),
        status,
    }
}

/// Is `name` already present in `dir` at the size oar-ocr expects?
///
/// Mirrors oar-ocr-core's own cache check so we never re-download a file
/// that the next [`crate::OcrEngine`] build would skip anyway. The sha256
/// verification is left to oar-ocr; reading every file each launch would
/// dwarf the actual OCR work.
fn is_cached(dir: &Path, name: &str) -> bool {
    let path = dir.join(name);
    match download::find(name) {
        Some(entry) => fs::metadata(&path)
            .map(|m| m.is_file() && m.len() == entry.size)
            .unwrap_or(false),
        None => path.is_file(),
    }
}

/// Live progress bar pinned to one model file.
///
/// oar-ocr downloads into `.{name}.{pid}.{counter}.part` then renames into
/// place on success, so we sum every `.part` file matching the prefix and
/// fall back to the final file once the rename happens.
fn progress_loop(dir: &Path, name: &str, total: u64, stop: &AtomicBool) {
    let prefix = format!(".{name}.");
    let target = dir.join(name);
    let mut last = 0u64;
    while !stop.load(Ordering::Acquire) {
        let cur = match fs::metadata(&target) {
            Ok(m) if m.is_file() => m.len(),
            _ => partial_size(dir, &prefix),
        };
        if cur != last {
            print_bar(name, cur, total);
            last = cur;
        }
        thread::sleep(Duration::from_millis(200));
    }
    // Final render so the bar always shows 100% on a successful download.
    let cur = fs::metadata(dir.join(name))
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .unwrap_or(last);
    print_bar(name, cur, total);
    let _ = writeln!(std::io::stderr());
}

fn partial_size(dir: &Path, prefix: &str) -> u64 {
    let Ok(rd) = fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in rd.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if name.starts_with(prefix) && name.ends_with(".part") {
            if let Ok(m) = entry.metadata() {
                total += m.len();
            }
        }
    }
    total
}

fn print_bar(name: &str, cur: u64, total: u64) {
    const WIDTH: usize = 24;
    let pct = if total == 0 {
        0.0
    } else {
        (cur as f64 / total as f64).clamp(0.0, 1.0)
    };
    let filled = (pct * WIDTH as f64).round() as usize;
    let bar: String = std::iter::repeat('#')
        .take(filled)
        .chain(std::iter::repeat('.').take(WIDTH - filled))
        .collect();
    let mut stderr = std::io::stderr();
    let _ = write!(
        stderr,
        "\r[OCR] {name} [{bar}] {} / {} ({:>5.1}%)   ",
        human_bytes(cur),
        human_bytes(total),
        pct * 100.0
    );
    let _ = stderr.flush();
}

fn human_bytes(b: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let f = b as f64;
    if f >= GB {
        format!("{:.2} GiB", f / GB)
    } else if f >= MB {
        format!("{:.1} MiB", f / MB)
    } else if f >= KB {
        format!("{:.1} KiB", f / KB)
    } else {
        format!("{b} B")
    }
}

