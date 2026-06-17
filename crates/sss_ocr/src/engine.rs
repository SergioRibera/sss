use std::path::PathBuf;
use std::sync::Arc;

use image::{DynamicImage, RgbImage, RgbaImage};
use oar_ocr::core::config::{OrtGraphOptimizationLevel, OrtSessionConfig};
use oar_ocr::domain::tasks::TextDetectionConfig;
use oar_ocr::oarocr::{OAROCR, OAROCRBuilder};
use oar_ocr::processors::LimitType;

use crate::error::OcrError;
use crate::gpu::GpuMode;
use crate::hardware::Tier;
use crate::registry::{Language, ModelSet, resolve_models};
use crate::types::{TextBox, TextPoint};

/// A ready-to-use OCR pipeline.
///
/// Construction loads the ONNX models from disk; it is intentionally heavy.
/// Reuse a single `OcrEngine` across captures whenever possible.
pub struct OcrEngine {
    inner: OAROCR,
    set: ModelSet,
}

impl OcrEngine {
    /// Builds a pipeline for `(tier, language)`. `formula` only takes
    /// effect at [`Tier::Heavy`] — see [`resolve_models`].
    pub fn new(
        tier: Tier,
        language: Language,
        formula: bool,
        gpu: GpuMode,
    ) -> Result<Self, OcrError> {
        crate::install_models_dir();
        let set = resolve_models(tier, language, formula);
        let inner = build_pipeline(&set, gpu)?;
        Ok(Self { inner, set })
    }

    /// Builds a pipeline from a pre-resolved [`ModelSet`].
    pub fn from_models(set: ModelSet, gpu: GpuMode) -> Result<Self, OcrError> {
        crate::install_models_dir();
        let inner = build_pipeline(&set, gpu)?;
        Ok(Self { inner, set })
    }

    pub fn models(&self) -> &ModelSet {
        &self.set
    }

    /// Runs the pipeline on a single RGBA image and returns boxes in the
    /// input image's pixel coordinates.
    ///
    /// The alpha channel is dropped — PaddleOCR works in RGB.
    pub fn run(&self, image: &RgbaImage) -> Result<Vec<TextBox>, OcrError> {
        let rgb = DynamicImage::ImageRgba8(image.clone()).to_rgb8();
        self.run_rgb(rgb)
    }

    /// Like [`Self::run`] but accepts an already-converted RGB image, avoiding
    /// the per-call clone + colourspace conversion.
    pub fn run_rgb(&self, image: RgbImage) -> Result<Vec<TextBox>, OcrError> {
        self.predict_tiled(image, 0, 0)
    }

    /// Run OCR on a cropped sub-image and translate every detected polygon
    /// by `(offset_x, offset_y)` so the boxes land in the caller's
    /// coordinate space (typically: the eager full-frame capture).
    pub fn run_with_offset(
        &self,
        image: &RgbaImage,
        offset_x: i32,
        offset_y: i32,
    ) -> Result<Vec<TextBox>, OcrError> {
        let rgb = DynamicImage::ImageRgba8(image.clone()).to_rgb8();
        self.predict_tiled(rgb, offset_x, offset_y)
    }

    /// Detection + recognition with automatic tiling.
    ///
    /// DBNet's feature pyramid scales its intermediate activations with
    /// the input area, so a single forward over an ultra-wide (3440×1440
    /// or 5120×2160) screenshot OOMs even on 8 GB GPUs. We cap each
    /// forward at `TILE_SIZE` on the long side by slicing the input into
    /// an overlapping grid, running the pipeline per tile, translating
    /// polygons into global coordinates and de-duping across tile seams
    /// with a polygon-bbox IoU pass. Single-pass when the input already
    /// fits — no overhead for 1080p / 1200p users.
    fn predict_tiled(
        &self,
        image: RgbImage,
        offset_x: i32,
        offset_y: i32,
    ) -> Result<Vec<TextBox>, OcrError> {
        let (w, h) = image.dimensions();
        let tiles = make_tiles(w, h, TILE_SIZE, TILE_OVERLAP);
        let mut all = Vec::new();
        if tiles.len() == 1 {
            let mut results = self.inner.predict(vec![image])?;
            if let Some(first) = results.pop() {
                all = map_regions(first.text_regions);
            }
        } else {
            tracing::debug!(
                tiles = tiles.len(),
                input_w = w,
                input_h = h,
                "tiled OCR detection"
            );
            for t in tiles {
                let sub =
                    image::imageops::crop_imm(&image, t.x, t.y, t.w, t.h).to_image();
                let mut results = self.inner.predict(vec![sub])?;
                let Some(first) = results.pop() else {
                    continue;
                };
                let mut boxes = map_regions(first.text_regions);
                if t.x != 0 || t.y != 0 {
                    translate(&mut boxes, t.x as f32, t.y as f32);
                }
                all.append(&mut boxes);
            }
            all = nms_merge(all, NMS_IOU);
        }
        if offset_x != 0 || offset_y != 0 {
            translate(&mut all, offset_x as f32, offset_y as f32);
        }
        Ok(all)
    }
}

fn build_pipeline(set: &ModelSet, gpu: GpuMode) -> Result<OAROCR, OcrError> {
    let providers = gpu.to_providers();
    tracing::info!(?gpu, ?providers, "OCR ORT execution-provider chain");
    let mut builder = OAROCRBuilder::new(
        PathBuf::from(set.detection),
        PathBuf::from(set.recognition),
        PathBuf::from(set.dict),
    )
    .ort_session(cpu_ort_session_config().with_execution_providers(providers))
    .text_detection_config(high_res_detection_config())
    // Recognition is the memory hog on dense text: the default region
    // batch (32+ on the adapter side) packs every detected polygon
    // into a single tensor, which for a code editor full of text
    // means a 60×3×48×320 batch and ~800 MB workspace per Concat.
    // Capping at 8 keeps the working set under ~200 MB while still
    // letting cuDNN amortise the launch overhead.
    .region_batch_size(8);
    if let Some(name) = set.doc_orientation {
        builder = builder.with_document_image_orientation_classification(PathBuf::from(name));
    }
    if let Some(name) = set.line_orientation {
        builder = builder.with_text_line_orientation_classification(PathBuf::from(name));
    }
    Ok(builder.build()?)
}

/// Tuned ORT session.
///
/// `oar-ocr` defaults to `Level1` graph optimisation and unspecified thread
/// counts, which on a multi-core desktop leaves a lot of inference perf on
/// the floor. We:
/// * crank optimisation to `Level3` (constant folding, layout opts, fused
///   ops) — the one-shot cost is paid during engine build, not per-run;
/// * pin intra-op threads to the host's available parallelism (oar-ocr's
///   own default already does this, but being explicit guarantees it
///   survives any future upstream default change);
/// * keep inter-op at 1 — running multiple graph nodes in parallel is a
///   net loss on a single-threaded request stream like ours, and frees
///   the CPU pool for intra-op parallelism instead;
/// * disable memory-pattern optimisation. It only helps when every run
///   uses the exact same input shape; our tiled detection feeds the
///   model crops of varying widths/heights, so the planner ends up
///   caching a separate allocation plan per shape and bloats RAM/VRAM
///   without speeding anything up.
fn cpu_ort_session_config() -> OrtSessionConfig {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    OrtSessionConfig::new()
        .with_intra_threads(cores)
        .with_inter_threads(1)
        .with_parallel_execution(false)
        .with_optimization_level(OrtGraphOptimizationLevel::Level3)
        .with_memory_pattern(false)
}

/// Defaults inside `oar-ocr` cap the longest side of the input at 960px,
/// which murders small terminal / IDE text on a modern display: a
/// 1920×1200 screenshot gets bilinear-shrunk by 2× before the detection
/// network ever sees it, so anything below ~14 pixels of vertical glyph
/// height drops out of the proposals.
///
/// Tightrope walk: bumping the cap also blows up the intermediate
/// activations of DBNet's feature pyramid — at 4096 the `Concat.16` node
/// allocates ~800 MB which OOMs on most consumer GPUs. We compromise on
/// `Max` at 2048: any 1080p / 1200p screenshot passes through unscaled
/// (good detection quality on real glyphs), 4K monitors get downscaled
/// to 2048×1152 (~1.87× — still a big improvement over the default 4×),
/// and the cropped re-OCR pass on the user-selected region recovers
/// any glyph that fell through the first pass at native resolution.
fn high_res_detection_config() -> TextDetectionConfig {
    TextDetectionConfig {
        score_threshold: 0.3,
        box_threshold: 0.6,
        unclip_ratio: 1.8,
        max_candidates: 1500,
        limit_side_len: Some(2048),
        limit_type: Some(LimitType::Max),
        max_side_len: Some(2048),
    }
}

/// Max long-side of each tile fed into the detection model. 2048 is the
/// largest value that keeps DBNet's `Concat.16` intermediate under the
/// VRAM budget of a typical 8 GB consumer GPU.
const TILE_SIZE: u32 = 2048;
/// Pixels of overlap between adjacent tiles. Big enough to fully contain
/// a couple of lines of typical UI / terminal text (≥ 24 px glyphs with
/// padding) so no detection is bisected at a seam; small enough that we
/// don't pay for it on every tile.
const TILE_OVERLAP: u32 = 256;
/// IoU threshold for the post-tile NMS pass. Two detections from
/// adjacent tiles describing the same glyph cluster sit in lockstep,
/// so 0.5 confidently de-dupes without merging genuinely close lines.
const NMS_IOU: f32 = 0.5;

#[derive(Clone, Copy, Debug)]
struct Tile {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

fn make_tiles(w: u32, h: u32, size: u32, overlap: u32) -> Vec<Tile> {
    if w <= size && h <= size {
        return vec![Tile { x: 0, y: 0, w, h }];
    }
    let xs = axis_origins(w, size, overlap);
    let ys = axis_origins(h, size, overlap);
    let mut out = Vec::with_capacity(xs.len() * ys.len());
    for &y in &ys {
        for &x in &xs {
            let tw = (w - x).min(size);
            let th = (h - y).min(size);
            out.push(Tile { x, y, w: tw, h: th });
        }
    }
    out
}

fn axis_origins(dim: u32, size: u32, overlap: u32) -> Vec<u32> {
    if dim <= size {
        return vec![0];
    }
    let stride = size.saturating_sub(overlap).max(1);
    let mut out = Vec::new();
    let mut x = 0u32;
    loop {
        out.push(x);
        if x + size >= dim {
            break;
        }
        let next = x + stride;
        if next + size > dim {
            out.push(dim - size);
            break;
        }
        x = next;
    }
    out
}

fn translate(boxes: &mut [TextBox], dx: f32, dy: f32) {
    for tb in boxes.iter_mut() {
        for p in tb.polygon.iter_mut() {
            p.x += dx;
            p.y += dy;
        }
    }
}

fn polygon_bbox(poly: &[TextPoint]) -> (f32, f32, f32, f32) {
    let mut x0 = f32::INFINITY;
    let mut y0 = f32::INFINITY;
    let mut x1 = f32::NEG_INFINITY;
    let mut y1 = f32::NEG_INFINITY;
    for p in poly {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    (x0, y0, x1, y1)
}

fn bbox_iou(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> f32 {
    let (ax0, ay0, ax1, ay1) = a;
    let (bx0, by0, bx1, by1) = b;
    let ix0 = ax0.max(bx0);
    let iy0 = ay0.max(by0);
    let ix1 = ax1.min(bx1);
    let iy1 = ay1.min(by1);
    let iw = (ix1 - ix0).max(0.0);
    let ih = (iy1 - iy0).max(0.0);
    let inter = iw * ih;
    if inter <= 0.0 {
        return 0.0;
    }
    let area_a = (ax1 - ax0).max(0.0) * (ay1 - ay0).max(0.0);
    let area_b = (bx1 - bx0).max(0.0) * (by1 - by0).max(0.0);
    let uni = area_a + area_b - inter;
    if uni <= 0.0 {
        0.0
    } else {
        inter / uni
    }
}

/// Greedy NMS over polygon bounding boxes. Keeps the highest-confidence
/// detection in each cluster; anything overlapping it at ≥ `thr` IoU
/// drops out. Linear in the small number of detections per tile-merged
/// frame so the O(n²) pairwise pass is fine.
fn nms_merge(mut boxes: Vec<TextBox>, thr: f32) -> Vec<TextBox> {
    boxes.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut keep: Vec<(TextBox, (f32, f32, f32, f32))> = Vec::with_capacity(boxes.len());
    for tb in boxes {
        let bb = polygon_bbox(&tb.polygon);
        if keep.iter().any(|(_, kb)| bbox_iou(*kb, bb) >= thr) {
            continue;
        }
        keep.push((tb, bb));
    }
    keep.into_iter().map(|(tb, _)| tb).collect()
}

fn map_regions(regions: Vec<oar_ocr::oarocr::TextRegion>) -> Vec<TextBox> {
    regions
        .into_iter()
        .filter_map(|r| {
            let text = r.text.as_ref().map(Arc::clone)?;
            let polygon: Vec<TextPoint> = r
                .bounding_box
                .points
                .iter()
                .map(|p| TextPoint { x: p.x, y: p.y })
                .collect();
            if polygon.len() < 3 {
                return None;
            }
            Some(TextBox {
                polygon,
                text: text.as_ref().to_owned(),
                confidence: r.confidence.unwrap_or(0.0),
                label: r
                    .label
                    .as_ref()
                    .map(|s| s.as_ref().to_owned())
                    .unwrap_or_else(|| "text".to_owned()),
            })
        })
        .collect()
}
