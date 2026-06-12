use std::path::PathBuf;
use std::sync::Arc;

use image::{DynamicImage, RgbImage, RgbaImage};
use oar_ocr::oarocr::{OAROCR, OAROCRBuilder};

use crate::error::OcrError;
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
    pub fn new(tier: Tier, language: Language, formula: bool) -> Result<Self, OcrError> {
        crate::install_models_dir();
        let set = resolve_models(tier, language, formula);
        let inner = build_pipeline(&set)?;
        Ok(Self { inner, set })
    }

    /// Builds a pipeline from a pre-resolved [`ModelSet`].
    pub fn from_models(set: ModelSet) -> Result<Self, OcrError> {
        crate::install_models_dir();
        let inner = build_pipeline(&set)?;
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
        let mut results = self.inner.predict(vec![image])?;
        let Some(first) = results.pop() else {
            return Ok(Vec::new());
        };
        Ok(map_regions(first.text_regions))
    }
}

fn build_pipeline(set: &ModelSet) -> Result<OAROCR, OcrError> {
    let mut builder = OAROCRBuilder::new(
        PathBuf::from(set.detection),
        PathBuf::from(set.recognition),
        PathBuf::from(set.dict),
    );
    if let Some(name) = set.doc_orientation {
        builder = builder.with_document_image_orientation_classification(PathBuf::from(name));
    }
    if let Some(name) = set.line_orientation {
        builder = builder.with_text_line_orientation_classification(PathBuf::from(name));
    }
    Ok(builder.build()?)
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
