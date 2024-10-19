use font_kit::error::{FontLoadingError, GlyphLoadingError, SelectionError};
use image::ImageError;
use notify_rust::error::Error as NotificationError;
use std::num::ParseIntError;

#[cfg(all(unix, not(target_os = "macos")))]
use notify_rust::ImageError as NotificationImageError;

use thiserror::Error;

#[derive(Debug, Error)]
#[error(transparent)]
pub enum ImagenGeneration {
    Color(#[from] ParseColor),
    Clipboard(#[from] arboard::Error),
    Background(#[from] Background),
    Font(#[from] FontError),
    Image(#[from] ImageError),
    Notification(#[from] NotificationError),
    #[cfg(all(unix, not(target_os = "macos")))]
    NotificationImage(#[from] NotificationImageError),
    #[error("{0}")]
    Custom(String),
}

unsafe impl Send for ImagenGeneration {}
unsafe impl Sync for ImagenGeneration {}

#[derive(Debug, Error)]
#[error(transparent)]
pub enum Background {
    Color(#[from] ParseColor),
    #[error("Cannot Parse Background from String")]
    CannotParse,
    #[error("Invalid format of String")]
    InvalidFormat,
    #[error("Invalid path of image")]
    InvalidPath,
}

unsafe impl Send for Background {}
unsafe impl Sync for Background {}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ParseColor {
    #[error("Invalid length of String")]
    InvalidLength,
    #[error("Invalid digit")]
    InvalidDigit,
    #[error("Error parsing number")]
    Parse(#[from] ParseIntError),
}

unsafe impl Send for ParseColor {}
unsafe impl Sync for ParseColor {}

#[derive(Debug, Error)]
#[error(transparent)]
pub enum FontError {
    SelectionError(#[from] SelectionError),
    FontLoadingError(#[from] FontLoadingError),
    GlyphLoading(#[from] GlyphLoadingError),
    #[error("Bad format at parse font: {0}")]
    BadFormat(String),
    #[error("Failed to get font by style: {0}")]
    LoadByStyle(String),
    #[error("Cannot get font height from fronts loaded")]
    GetHeight,
}

unsafe impl Send for FontError {}
unsafe impl Sync for FontError {}
