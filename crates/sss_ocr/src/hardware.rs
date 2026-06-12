use serde::{Deserialize, Serialize};
use sysinfo::System;

/// Performance tier the OCR pipeline is allowed to use.
///
/// Tier picks the model variants (mobile vs server, formula on/off) and
/// indirectly the on-disk footprint. `Auto` defers to [`resolve_tier`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    #[default]
    Auto,
    /// Mobile detection + multilingual mobile recognition. ~25 MB total.
    Light,
    /// Server detection + mobile recognition. ~100 MB total.
    Standard,
    /// Server detection + server recognition + orientation models. ~200 MB.
    Heavy,
}

impl Tier {
    /// Resolves `Auto` against the host hardware. All other variants pass through.
    pub fn resolve(self) -> Tier {
        match self {
            Tier::Auto => resolve_tier(),
            t => t,
        }
    }

    /// Total RAM threshold (MB) above which this tier is comfortable.
    pub fn min_ram_mb(self) -> u64 {
        match self {
            Tier::Auto | Tier::Light => 0,
            Tier::Standard => 4_000,
            Tier::Heavy => 8_000,
        }
    }

    /// Logical CPU threshold above which this tier is comfortable.
    pub fn min_cores(self) -> usize {
        match self {
            Tier::Auto | Tier::Light => 0,
            Tier::Standard => 4,
            Tier::Heavy => 8,
        }
    }
}

/// Picks a concrete tier from host hardware: cores + total RAM.
///
/// Heuristic, intentionally simple. GPU detection is skipped on purpose —
/// onnxruntime CPU is the only execution provider we ship by default.
pub fn resolve_tier() -> Tier {
    let cores = num_cpus::get();
    let mut sys = System::new();
    sys.refresh_memory();
    let ram_mb = sys.total_memory() / 1024 / 1024;

    if cores < 4 || ram_mb < 4_000 {
        Tier::Light
    } else if cores < 8 || ram_mb < 8_000 {
        Tier::Standard
    } else {
        Tier::Heavy
    }
}
