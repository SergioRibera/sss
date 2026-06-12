//! Maps `(tier, language)` to a concrete set of PP-OCRv5 model filenames.
//!
//! The filenames here are the bare names that oar-ocr's `auto-download`
//! feature looks up in its bundled registry. They live under `OAR_HOME`
//! after the first download — see [`crate::install_models_dir`].

use serde::{Deserialize, Serialize};

use crate::hardware::Tier;

/// Logical recognition language. Drives recogniser + dictionary choice.
///
/// `Auto` means: use the broadest model that ships in PaddleOCR's PP-OCRv5
/// family (general/Chinese model — covers CJK + ASCII fine for screenshots
/// and is what PaddleOCR recommends when the input language is unknown).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Auto,
    Chinese,
    English,
    Latin,
    Korean,
    Cyrillic,
    Arabic,
    Greek,
    Devanagari,
    Tamil,
    Telugu,
    Thai,
    EasternSlavic,
}

impl Language {
    /// Returns true if this language uses the general PP-OCRv5 recogniser
    /// (i.e. no language-specific recogniser swap).
    pub fn uses_general_recogniser(self) -> bool {
        matches!(self, Language::Auto | Language::Chinese)
    }
}

/// Parses `["en", "es", ...]` → `Vec<Language>`, deduplicating in input order.
///
/// Unknown codes resolve to `Auto` and a warning is traced. An empty input
/// returns `[Language::Auto]`.
pub fn resolve_language(codes: &[String]) -> Vec<Language> {
    if codes.is_empty() {
        return vec![Language::Auto];
    }
    let mut out: Vec<Language> = Vec::with_capacity(codes.len());
    for raw in codes {
        let lang = parse_one(raw);
        if !out.contains(&lang) {
            out.push(lang);
        }
    }
    out
}

fn parse_one(code: &str) -> Language {
    match code.to_ascii_lowercase().as_str() {
        "" | "auto" | "any" => Language::Auto,
        "zh" | "ch" | "chinese" | "zh-cn" | "zh-tw" => Language::Chinese,
        "en" | "english" => Language::English,
        "es" | "fr" | "de" | "it" | "pt" | "nl" | "ca" | "pl" | "ro" | "sv" | "no" | "da"
        | "fi" | "tr" | "vi" | "id" | "ms" | "latin" => Language::Latin,
        "ko" | "kr" | "korean" => Language::Korean,
        "ru" | "uk" | "be" | "bg" | "sr" | "cyrillic" => Language::Cyrillic,
        "ar" | "fa" | "ur" | "arabic" => Language::Arabic,
        "el" | "greek" => Language::Greek,
        "hi" | "mr" | "ne" | "devanagari" => Language::Devanagari,
        "ta" | "tamil" => Language::Tamil,
        "te" | "telugu" => Language::Telugu,
        "th" | "thai" => Language::Thai,
        "eslav" | "eastern_slavic" => Language::EasternSlavic,
        other => {
            tracing::warn!(language = other, "unknown OCR language code; falling back to auto");
            Language::Auto
        }
    }
}

/// The concrete set of model files needed to run a pipeline at a given tier
/// and recognition language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSet {
    pub detection: &'static str,
    pub recognition: &'static str,
    pub dict: &'static str,
    pub doc_orientation: Option<&'static str>,
    pub line_orientation: Option<&'static str>,
    pub formula: Option<&'static str>,
    pub formula_tokenizer: Option<&'static str>,
}

impl ModelSet {
    /// Iterator over every file name in this set, including optional ones
    /// that are `Some`.
    pub fn files(&self) -> impl Iterator<Item = &'static str> {
        [
            Some(self.detection),
            Some(self.recognition),
            Some(self.dict),
            self.doc_orientation,
            self.line_orientation,
            self.formula,
            self.formula_tokenizer,
        ]
        .into_iter()
        .flatten()
    }
}

/// Picks a [`ModelSet`] for the given tier + language + formula opt-in.
///
/// `Tier::Auto` is resolved against the host. The recognition model swaps
/// with `language`; detection and (optional) orientation models are tied
/// to the tier.
pub fn resolve_models(tier: Tier, language: Language, formula: bool) -> ModelSet {
    let tier = tier.resolve();
    let (detection, doc_orientation, line_orientation) = match tier {
        Tier::Light => ("pp-ocrv5_mobile_det.onnx", None, None),
        Tier::Standard => (
            "pp-ocrv5_server_det.onnx",
            None,
            Some("pp-lcnet_x0_25_textline_ori.onnx"),
        ),
        Tier::Heavy => (
            "pp-ocrv5_server_det.onnx",
            Some("pp-lcnet_x1_0_doc_ori.onnx"),
            Some("pp-lcnet_x1_0_textline_ori.onnx"),
        ),
        // Should not happen: Auto::resolve guarantees a concrete tier.
        Tier::Auto => unreachable!("Tier::Auto must be resolved before reaching resolve_models"),
    };

    let (recognition, dict) = recogniser_for(language, tier);

    let (formula, formula_tokenizer) = if formula && matches!(tier, Tier::Heavy) {
        (Some("pp-formulanet_plus-s.onnx"), None)
    } else {
        (None, None)
    };

    ModelSet {
        detection,
        recognition,
        dict,
        doc_orientation,
        line_orientation,
        formula,
        formula_tokenizer,
    }
}

fn recogniser_for(language: Language, tier: Tier) -> (&'static str, &'static str) {
    // Server-tier general recogniser is the only place we use the heavy
    // recognition model; everywhere else we pick mobile so the download
    // footprint stays sensible.
    if language.uses_general_recogniser() {
        return match tier {
            Tier::Heavy => ("pp-ocrv5_server_rec.onnx", "ppocrv5_dict.txt"),
            _ => ("pp-ocrv5_mobile_rec.onnx", "ppocrv5_dict.txt"),
        };
    }

    match language {
        Language::English => ("en_pp-ocrv5_mobile_rec.onnx", "ppocrv5_en_dict.txt"),
        Language::Latin => ("latin_pp-ocrv5_mobile_rec.onnx", "ppocrv5_latin_dict.txt"),
        Language::Korean => ("korean_pp-ocrv5_mobile_rec.onnx", "ppocrv5_korean_dict.txt"),
        Language::Cyrillic => (
            "cyrillic_pp-ocrv5_mobile_rec.onnx",
            "ppocrv5_cyrillic_dict.txt",
        ),
        Language::Arabic => (
            "arabic_pp-ocrv5_mobile_rec.onnx",
            "ppocrv5_arabic_dict.txt",
        ),
        Language::Greek => ("el_pp-ocrv5_mobile_rec.onnx", "ppocrv5_el_dict.txt"),
        Language::Devanagari => (
            "devanagari_pp-ocrv5_mobile_rec.onnx",
            "ppocrv5_devanagari_dict.txt",
        ),
        Language::Tamil => ("ta_pp-ocrv5_mobile_rec.onnx", "ppocrv5_ta_dict.txt"),
        Language::Telugu => ("te_pp-ocrv5_mobile_rec.onnx", "ppocrv5_te_dict.txt"),
        Language::Thai => ("th_pp-ocrv5_mobile_rec.onnx", "ppocrv5_th_dict.txt"),
        Language::EasternSlavic => (
            "eslav_pp-ocrv5_mobile_rec.onnx",
            "ppocrv5_eslav_dict.txt",
        ),
        Language::Auto | Language::Chinese => unreachable!("handled above"),
    }
}

/// Returns the union of every model file required to satisfy a list of
/// languages at the given tier. Used by [`crate::prewarm`] to drive
/// downloads of every recogniser the user opted into.
pub fn union_files(tier: Tier, languages: &[Language], formula: bool) -> Vec<&'static str> {
    let tier = tier.resolve();
    let mut seen: Vec<&'static str> = Vec::new();
    for lang in languages {
        let set = resolve_models(tier, *lang, formula);
        for f in set.files() {
            if !seen.contains(&f) {
                seen.push(f);
            }
        }
    }
    seen
}
