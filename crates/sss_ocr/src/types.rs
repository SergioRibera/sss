//! Re-exports the shared OCR types defined in `sss_core::ocr`.
//!
//! Kept as a thin module so the rest of `sss_ocr` can keep importing from
//! `crate::types` while the canonical definitions live in `sss_core` and
//! are reusable by `sss_capture_ui` without a back-dep on `sss_ocr`.

pub use sss_core::ocr::{TextBox, TextPoint};
