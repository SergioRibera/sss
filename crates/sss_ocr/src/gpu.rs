//! Execution-provider selection for the OCR engine.
//!
//! Maps a user-facing `GpuMode` (CLI flag / TOML setting) to the list of
//! [`OrtExecutionProvider`]s that the ORT session is configured with.
//!
//! Ordering matters: ORT walks the list and uses the first provider that
//! actually has support compiled into the underlying `libonnxruntime`.
//! We always append `CPU` last so a CUDA-only build with no GPU available
//! still resolves to a functional pipeline.

use oar_ocr::core::config::OrtExecutionProvider;
use serde::{Deserialize, Serialize};

/// How OCR inference should be accelerated.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum GpuMode {
    /// Pick the best-effort default for the host: CoreML on macOS, CUDA
    /// when the binary was built with the `cuda` feature *and* the host
    /// exposes `/dev/nvidia0`, otherwise CPU.
    #[default]
    Auto,
    /// Force CPU. Useful when the user has a flaky GPU driver and wants
    /// reproducible behaviour.
    Cpu,
    /// NVIDIA CUDA. Requires the `cuda` Cargo feature and a CUDA-enabled
    /// `libonnxruntime`.
    Cuda,
    /// NVIDIA TensorRT. Higher peak throughput than CUDA but with a much
    /// longer first-run warmup; pair with `engine_cache=true` in any
    /// long-lived deployment.
    TensorRT,
    /// Apple CoreML / Neural Engine. macOS only.
    CoreML,
    /// Windows DirectML. GPU-vendor agnostic on Win10+.
    DirectML,
    /// Intel OpenVINO. CPU + iGPU + Movidius.
    OpenVino,
    /// WebGPU EP. Cross-vendor but newer; treat as experimental.
    WebGpu,
}

impl GpuMode {
    /// Map the mode to an ORT provider chain. `CPU` is always appended so
    /// a missing GPU EP falls back to software inference instead of
    /// crashing.
    pub fn to_providers(self) -> Vec<OrtExecutionProvider> {
        let resolved = match self {
            GpuMode::Auto => auto_pick(),
            other => other,
        };
        let mut out = Vec::with_capacity(2);
        match resolved {
            GpuMode::Cpu | GpuMode::Auto => {}
            // Tight memory budget. ORT's BFC arena defaults to
            // `NextPowerOfTwo` (an 800 MB tensor reserves 1 GiB; per-tile
            // shape variability then keeps the high-water mark climbing
            // until VRAM is gone — observed at ~9 GB on a 1080p
            // screenshot). `SameAsRequested` allocates only what each
            // session.run actually asks for, and the `gpu_mem_limit`
            // caps the arena hard. Combined with cuDNN heuristic algo
            // search (skips the workspace-hungry exhaustive sweep) the
            // peak drops to ~1-2 GiB without affecting OCR quality.
            GpuMode::Cuda => out.push(OrtExecutionProvider::CUDA {
                device_id: Some(0),
                // 4 GiB cap: detection workspace + recognition batch
                // both fit. Lower (2 GiB) starves the recognition
                // workspace when a tile yields many text crops; higher
                // is unnecessary and steals VRAM from the rest of the
                // session (compositor, GPU UI).
                gpu_mem_limit: Some(4 * 1024 * 1024 * 1024),
                arena_extend_strategy: Some("SameAsRequested".into()),
                cudnn_conv_algo_search: Some("Heuristic".into()),
                cudnn_conv_use_max_workspace: Some(false),
            }),
            GpuMode::TensorRT => out.push(OrtExecutionProvider::TensorRT {
                device_id: Some(0),
                max_workspace_size: None,
                min_subgraph_size: None,
                fp16_enable: Some(true),
                timing_cache: Some(true),
                timing_cache_path: None,
                force_timing_cache: None,
                engine_cache: Some(true),
                engine_cache_path: None,
                dump_ep_context_model: None,
                ep_context_file_path: None,
            }),
            GpuMode::CoreML => out.push(OrtExecutionProvider::CoreML {
                ane_only: None,
                subgraphs: Some(true),
            }),
            GpuMode::DirectML => out.push(OrtExecutionProvider::DirectML {
                device_id: Some(0),
            }),
            GpuMode::OpenVino => out.push(OrtExecutionProvider::OpenVINO {
                device_type: None,
                num_threads: None,
            }),
            GpuMode::WebGpu => out.push(OrtExecutionProvider::WebGPU),
        }
        out.push(OrtExecutionProvider::CPU);
        out
    }
}

/// Pick the best EP for the host.
///
/// Compile-time feature gates whittle this down: if the binary wasn't
/// built with `cuda`, asking for CUDA at runtime can't succeed anyway, so
/// we don't probe.
fn auto_pick() -> GpuMode {
    #[cfg(all(target_os = "macos", feature = "coreml"))]
    {
        return GpuMode::CoreML;
    }
    #[cfg(all(target_os = "windows", feature = "directml"))]
    {
        return GpuMode::DirectML;
    }
    #[cfg(all(target_os = "linux", feature = "cuda"))]
    {
        if std::path::Path::new("/dev/nvidia0").exists() {
            return GpuMode::Cuda;
        }
    }
    GpuMode::Cpu
}
