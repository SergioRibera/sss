use thiserror::Error;

#[derive(Debug, Error)]
pub enum OcrError {
    #[error("oar-ocr error: {0}")]
    Oar(String),

    #[error("OCR is not enabled by configuration")]
    Disabled,

    #[error("OCR models are still downloading")]
    NotReady,

    #[error("language `{0}` is not supported")]
    UnsupportedLanguage(String),

    #[error("model file `{0}` was not found at `{1}`")]
    MissingModel(String, std::path::PathBuf),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<oar_ocr::core::OCRError> for OcrError {
    fn from(value: oar_ocr::core::OCRError) -> Self {
        OcrError::Oar(value.to_string())
    }
}
