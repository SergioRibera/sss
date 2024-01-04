use image::Rgba;

use crate::error::ParseColor as ParseColorError;

pub trait ToRgba {
    type Target;
    fn to_rgba(&self) -> Self::Target;
}

/// Parse hex color (#RRGGBB or #RRGGBBAA)
impl ToRgba for String {
    type Target = Result<Rgba<u8>, ParseColorError>;

    fn to_rgba(&self) -> Self::Target {
        if self.as_bytes()[0] != b'#' {
            return Err(ParseColorError::InvalidDigit);
        }
        let mut color = u32::from_str_radix(&self[1..], 16)?;

        match self.len() {
            // RGB or RGBA
            4 | 5 => {
                let a = if self.len() == 5 {
                    let alpha = (color & 0xf) as u8;
                    color >>= 4;
                    alpha
                } else {
                    0xff
                };

                let r = ((color >> 8) & 0xf) as u8;
                let g = ((color >> 4) & 0xf) as u8;
                let b = (color & 0xf) as u8;

                Ok(Rgba([r << 4 | r, g << 4 | g, b << 4 | b, a << 4 | a]))
            }
            // RRGGBB or RRGGBBAA
            7 | 9 => {
                let alpha = if self.len() == 9 {
                    let alpha = (color & 0xff) as u8;
                    color >>= 8;
                    alpha
                } else {
                    0xff
                };

                Ok(Rgba([
                    (color >> 16) as u8,
                    (color >> 8) as u8,
                    color as u8,
                    alpha,
                ]))
            }
            _ => Err(ParseColorError::InvalidLength),
        }
    }
}

impl ToRgba for str {
    type Target = Result<Rgba<u8>, ParseColorError>;

    fn to_rgba(&self) -> Self::Target {
        String::from(self).to_rgba()
    }
}
