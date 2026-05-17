//! Canonical hex color parser shared across crates.

use std::num::ParseIntError;

use image::Rgba;
use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ParseColor {
    #[error("Invalid length of String")]
    InvalidLength,
    #[error("Invalid digit")]
    InvalidDigit,
    #[error("Error parsing number")]
    Parse(#[from] ParseIntError),
}

/// Parse `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA` into RGBA bytes. The `#`
/// prefix is optional; surrounding whitespace is ignored. Short forms expand
/// each nibble to a full byte (`F` → `0xFF`).
pub fn parse_hex(s: &str) -> Result<[u8; 4], ParseColor> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ParseColor::InvalidLength);
    }
    let body = s.strip_prefix('#').unwrap_or(s);
    let nibble = |i: usize| u8::from_str_radix(&body[i..i + 1], 16).map_err(ParseColor::from);
    let pair = |i: usize| u8::from_str_radix(&body[i..i + 2], 16).map_err(ParseColor::from);
    match body.len() {
        3 => {
            let r = nibble(0)?;
            let g = nibble(1)?;
            let b = nibble(2)?;
            Ok([r * 17, g * 17, b * 17, 255])
        }
        4 => {
            let r = nibble(0)?;
            let g = nibble(1)?;
            let b = nibble(2)?;
            let a = nibble(3)?;
            Ok([r * 17, g * 17, b * 17, a * 17])
        }
        6 => Ok([pair(0)?, pair(2)?, pair(4)?, 255]),
        8 => Ok([pair(0)?, pair(2)?, pair(4)?, pair(6)?]),
        _ => Err(ParseColor::InvalidLength),
    }
}

/// Compact hex form (`#RRGGBB` when fully opaque, otherwise `#RRGGBBAA`).
pub fn to_hex(rgba: [u8; 4]) -> String {
    let [r, g, b, a] = rgba;
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

pub trait ToRgba {
    type Target;
    fn to_rgba(&self) -> Self::Target;
}

/// Strict parser kept for `sss_lib` back-compat: requires a leading `#`.
impl ToRgba for str {
    type Target = Result<Rgba<u8>, ParseColor>;
    fn to_rgba(&self) -> Self::Target {
        if !self.starts_with('#') {
            return Err(ParseColor::InvalidDigit);
        }
        parse_hex(self).map(Rgba)
    }
}

impl ToRgba for String {
    type Target = Result<Rgba<u8>, ParseColor>;
    fn to_rgba(&self) -> Self::Target {
        self.as_str().to_rgba()
    }
}
