use font_kit::error::{FontLoadingError, SelectionError};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CodeScreenshotError {
    #[error("The expected format for {0} is {1}")]
    InvalidFormat(&'static str, &'static str),
    #[error("Font error")]
    Font(#[from] FontError),
}

#[derive(Debug, Error)]
#[error(transparent)]
pub enum FontError {
    SelectionError(#[from] SelectionError),
    FontLoadingError(#[from] FontLoadingError),
}
