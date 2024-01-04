use font_kit::error::{FontLoadingError, SelectionError};
use std::error::Error;
use std::fmt::{self, Display};
use std::num::ParseIntError;

#[derive(Debug)]
pub enum ImagenGeneration {
    Font(Font),
    Color(ParseColor),
}

#[derive(Debug)]
pub enum Font {
    SelectionError(SelectionError),
    FontLoadingError(FontLoadingError),
}

impl Error for Font {}

impl Display for Font {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Font::SelectionError(e) => write!(f, "Font error: {}", e),
            Font::FontLoadingError(e) => write!(f, "Font error: {}", e),
        }
    }
}

impl From<SelectionError> for Font {
    fn from(e: SelectionError) -> Self {
        Font::SelectionError(e)
    }
}

impl From<FontLoadingError> for Font {
    fn from(e: FontLoadingError) -> Self {
        Font::FontLoadingError(e)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum ParseColor {
    InvalidLength,
    InvalidDigit,
}

impl Error for ParseColor {}

impl Display for ParseColor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseColor::InvalidDigit => write!(f, "Invalid digit"),
            ParseColor::InvalidLength => write!(f, "Invalid length"),
        }
    }
}

impl From<ParseIntError> for ParseColor {
    fn from(_e: ParseIntError) -> Self {
        ParseColor::InvalidDigit
    }
}
