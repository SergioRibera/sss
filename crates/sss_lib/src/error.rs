use font_kit::error::{FontLoadingError, GlyphLoadingError, SelectionError};
use image::ImageError;
use notify_rust::{error::Error as NotificationError, ImageError as NotificationImageError};
use std::error::Error;
use std::fmt::{self, Write};
use std::num::ParseIntError;

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
    NotificationImage(#[from] NotificationImageError),
}

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

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ParseColor {
    #[error("Invalid length of String")]
    InvalidLength,
    #[error("Invalid digit")]
    InvalidDigit,
    #[error("Error parsing number")]
    Parse(#[from] ParseIntError),
}

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

// this code is inspiration from https://github.com/Kijewski/pretty-error-debug/blob/main/src/implementation.rs
/// Wrap an [`Error`] to display its error chain in debug messages ([`format!("{:?}")`][fmt::Debug]).
///
/// ```
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrettyErrorWrapper<E: 'static + Error>(pub E);

impl<E: 'static + Error> PrettyErrorWrapper<E> {
    /// Return the wrapped argument.
    #[inline]
    pub fn new(err: E) -> Self {
        Self(err)
    }
}

impl<E: 'static + Error> From<E> for PrettyErrorWrapper<E> {
    #[inline]
    fn from(value: E) -> Self {
        Self(value)
    }
}

impl<E: 'static + Error> Error for PrettyErrorWrapper<E> {
    #[inline]
    fn source(&self) -> Option<&(dyn 'static + Error)> {
        Some(&self.0)
    }
}

impl<E: 'static + Error> fmt::Display for PrettyErrorWrapper<E> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl<E: 'static + Error> fmt::Debug for PrettyErrorWrapper<E> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let error = &self.0;
        write!(f, "{}", error)?;
        if let Some(cause) = error.source() {
            write!(f, "\n\nCaused by:")?;
            let multiple = cause.source().is_some();
            for (n, error) in Chain(Some(cause)).enumerate() {
                writeln!(f)?;
                let mut indented = Indented {
                    inner: f,
                    number: if multiple { Some(n + 1) } else { None },
                    started: false,
                };
                write!(indented, "{}", error)?;
            }
        }
        Ok(())
    }
}

struct Chain<'a>(Option<&'a dyn Error>);

impl<'a> Iterator for Chain<'a> {
    type Item = &'a dyn Error;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let error = self.0?;
        self.0 = error.source();
        Some(error)
    }
}

struct Indented<'a, 'b: 'a> {
    inner: &'a mut fmt::Formatter<'b>,
    number: Option<usize>,
    started: bool,
}

impl<'a, 'b: 'a> fmt::Write for Indented<'a, 'b> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for (i, line) in s.split('\n').enumerate() {
            if !self.started {
                self.started = true;
                match self.number {
                    Some(number) => write!(self.inner, "{: >5}: ", number)?,
                    None => self.inner.write_str("    ")?,
                }
            } else if i > 0 {
                self.inner.write_char('\n')?;
                if self.number.is_some() {
                    self.inner.write_str("       ")?;
                } else {
                    self.inner.write_str("    ")?;
                }
            }

            self.inner.write_str(line)?;
        }

        Ok(())
    }
}
