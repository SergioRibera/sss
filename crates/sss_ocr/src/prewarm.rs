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
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use oar_ocr::download;

use crate::error::OcrError;
use crate::hardware::Tier;
use crate::registry::{Language, union_files};

/// Default download concurrency. ModelScope throttles per-connection on
/// most edges; 4 parallel fetches saturate residential links without
/// tripping per-IP rate limits.
const DEFAULT_PARALLEL: usize = 4;
/// Hard upper bound to keep accidental `SSS_OCR_PARALLEL=999` from
/// stampeding the mirror.
const MAX_PARALLEL: usize = 12;

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
            let parallelism = resolve_parallelism(pending.len());
            eprintln!(
                "[OCR] downloading {} model file(s), {} total, {} parallel → {}",
                pending.len(),
                human_bytes(total_bytes),
                parallelism,
                dir.display()
            );

            // Shared work queue + counters. The queue is drained from the
            // back by workers (cheap `Vec::pop`); the order doesn't matter
            // because every file is independent.
            let queue: Arc<Mutex<Vec<&'static str>>> =
                Arc::new(Mutex::new(pending.clone()));
            let done_count = Arc::new(AtomicUsize::new(0));
            let failure: Arc<Mutex<Option<OcrError>>> = Arc::new(Mutex::new(None));
            let stop_watch = Arc::new(AtomicBool::new(false));

            // Single aggregate progress watcher — printing per-file bars
            // from N parallel workers would clobber each other on stderr.
            let watch_dir = dir.clone();
            let watch_pending = pending.clone();
            let watch_total = total_bytes;
            let watch_count = pending.len();
            let watch_done = Arc::clone(&done_count);
            let watch_stop = Arc::clone(&stop_watch);
            let watcher = thread::Builder::new()
                .name("sss-ocr-progress".into())
                .spawn(move || {
                    aggregate_loop(
                        &watch_dir,
                        &watch_pending,
                        watch_total,
                        watch_count,
                        &watch_done,
                        &watch_stop,
                    )
                })
                .expect("failed to spawn aggregate progress watcher");

            let mut workers = Vec::with_capacity(parallelism);
            for i in 0..parallelism {
                let queue = Arc::clone(&queue);
                let done_count = Arc::clone(&done_count);
                let failure = Arc::clone(&failure);
                workers.push(
                    thread::Builder::new()
                        .name(format!("sss-ocr-fetch-{i}"))
                        .spawn(move || {
                            loop {
                                // Bail early if a sibling worker already
                                // hit an error — no point pulling more
                                // files we're about to discard.
                                if failure
                                    .lock()
                                    .map(|g| g.is_some())
                                    .unwrap_or(false)
                                {
                                    return;
                                }
                                let next = queue
                                    .lock()
                                    .expect("download queue poisoned")
                                    .pop();
                                let Some(file) = next else {
                                    return;
                                };
                                match download::fetch(file) {
                                    Ok(_) => {
                                        done_count
                                            .fetch_add(1, Ordering::AcqRel);
                                    }
                                    Err(e) => {
                                        let mut slot = failure
                                            .lock()
                                            .expect("failure mutex poisoned");
                                        if slot.is_none() {
                                            *slot = Some(OcrError::from(e));
                                        }
                                        return;
                                    }
                                }
                            }
                        })
                        .expect("failed to spawn fetch worker"),
                );
            }
            for w in workers {
                let _ = w.join();
            }
            stop_watch.store(true, Ordering::Release);
            let _ = watcher.join();

            if let Some(err) = failure
                .lock()
                .expect("failure mutex poisoned")
                .take()
            {
                eprintln!("[OCR] download failed: {err}");
                status_thread
                    .store(PrewarmStatus::Failed as u8, Ordering::Release);
                return Err(err);
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

/// Resolve worker count.
///
/// Honours `SSS_OCR_PARALLEL`, clamped to `[1, MAX_PARALLEL]`, and bounded
/// by the number of files so we never spawn idle workers.
fn resolve_parallelism(file_count: usize) -> usize {
    let requested = std::env::var("SSS_OCR_PARALLEL")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_PARALLEL);
    requested.clamp(1, MAX_PARALLEL).min(file_count.max(1))
}

/// Live aggregate progress bar across every file currently downloading.
///
/// Per-file bars would clobber each other from N parallel workers, so we
/// instead summarise: completed-files / total-files, bytes-on-disk /
/// total-bytes, and a single percentage. Polled every 300 ms; the cost is
/// one `read_dir` per tick.
fn aggregate_loop(
    dir: &Path,
    pending: &[&'static str],
    total: u64,
    count: usize,
    done: &AtomicUsize,
    stop: &AtomicBool,
) {
    while !stop.load(Ordering::Acquire) {
        let bytes = aggregate_bytes(dir, pending);
        let d = done.load(Ordering::Acquire);
        print_aggregate(d, count, bytes, total);
        thread::sleep(Duration::from_millis(300));
    }
    let bytes = aggregate_bytes(dir, pending);
    let d = done.load(Ordering::Acquire);
    print_aggregate(d, count, bytes, total);
    let _ = writeln!(std::io::stderr());
}

/// Sum the bytes on disk across every `pending` file — counting the final
/// path first, falling back to the `.part` tmp files oar-ocr writes during
/// transfer (see `download_and_verify` upstream).
fn aggregate_bytes(dir: &Path, pending: &[&'static str]) -> u64 {
    let mut total = 0u64;
    for name in pending {
        let target = dir.join(name);
        if let Ok(m) = fs::metadata(&target) {
            if m.is_file() {
                total += m.len();
                continue;
            }
        }
        let prefix = format!(".{name}.");
        total += partial_size(dir, &prefix);
    }
    total
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

fn print_aggregate(done: usize, count: usize, bytes: u64, total: u64) {
    const WIDTH: usize = 30;
    let pct = if total == 0 {
        0.0
    } else {
        (bytes as f64 / total as f64).clamp(0.0, 1.0)
    };
    let filled = (pct * WIDTH as f64).round() as usize;
    let bar: String = std::iter::repeat('#')
        .take(filled)
        .chain(std::iter::repeat('.').take(WIDTH - filled))
        .collect();
    let mut stderr = std::io::stderr();
    let _ = write!(
        stderr,
        "\r[OCR] [{bar}] {done}/{count} files  {} / {}  ({:>5.1}%)   ",
        human_bytes(bytes),
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

