//! A basic font manager with fallback support
//!
//! code modified from https://github.com/Aloxaf/silicon/blob/447c456385136abf5605e42e7c6e71acd3fdcd0d/src/font.rs
use conv::ValueInto;
use font_kit::canvas::{Canvas, Format, RasterizationOptions};
use font_kit::font::Font;
use font_kit::hinting::HintingOptions;
use font_kit::properties::{Properties, Style, Weight};
use font_kit::source::SystemSource;
use image::{GenericImage, Pixel};
use imageproc::definitions::Clamp;
use imageproc::pixelops::weighted_sum;
use pathfinder_geometry::transform2d::Transform2F;
use serde::de::{Deserialize, Deserializer};
use serde::{Serialize, Serializer};
use std::collections::HashMap;
use std::sync::Arc;

/// Font style
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum FontStyle {
    Regular,
    Italic,
    Bold,
    BoldItalic,
}

use pathfinder_geometry::rect::RectI;
use pathfinder_geometry::vector::Vector2I;
use FontStyle::*;

use crate::error::FontError;

/// A single font with specific size
#[derive(Clone, Debug)]
pub struct ImageFont {
    pub name: String,
    pub fonts: HashMap<FontStyle, Font>,
    pub size: f32,
}

unsafe impl Send for ImageFont {}
unsafe impl Sync for ImageFont {}

impl Default for ImageFont {
    /// It will use Hack font (size: 26.0) by default
    fn default() -> Self {
        let l = vec![
            (
                Regular,
                include_bytes!("../../../assets/fonts/Hack-Regular.ttf").to_vec(),
            ),
            (
                Italic,
                include_bytes!("../../../assets/fonts/Hack-Italic.ttf").to_vec(),
            ),
            (
                Bold,
                include_bytes!("../../../assets/fonts/Hack-Bold.ttf").to_vec(),
            ),
            (
                BoldItalic,
                include_bytes!("../../../assets/fonts/Hack-BoldItalic.ttf").to_vec(),
            ),
        ];
        let mut fonts = HashMap::new();
        for (style, bytes) in l {
            let font = Font::from_bytes(Arc::new(bytes), 0).unwrap();
            fonts.insert(style, font);
        }

        Self {
            name: "Hack".to_string(),
            fonts,
            size: 26.0,
        }
    }
}

impl ImageFont {
    pub fn new(name: &str, size: f32) -> Result<Self, FontError> {
        // Silicon already contains Hack font
        if name == "Hack" {
            let font = ImageFont {
                size,
                ..Default::default()
            };
            return Ok(font);
        }

        let mut fonts = HashMap::new();

        let family = SystemSource::new().select_family_by_name(name)?;
        let handles = family.fonts();

        log::debug!("{:?}", handles);

        for handle in handles {
            let font = handle.load()?;

            let properties: Properties = font.properties();

            log::debug!("{:?} - {:?}", font, properties);

            // cannot use match because `Weight` didn't derive `Eq`
            match properties.style {
                Style::Normal => {
                    if properties.weight == Weight::NORMAL {
                        fonts.insert(Regular, font);
                    } else if properties.weight == Weight::BOLD {
                        fonts.insert(Bold, font);
                    } else if properties.weight == Weight::MEDIUM && !fonts.contains_key(&Regular) {
                        fonts.insert(Regular, font);
                    }
                }
                Style::Italic => {
                    if properties.weight == Weight::NORMAL {
                        fonts.insert(Italic, font);
                    } else if properties.weight == Weight::BOLD {
                        fonts.insert(BoldItalic, font);
                    } else if properties.weight == Weight::MEDIUM && !fonts.contains_key(&Italic) {
                        fonts.insert(Italic, font);
                    }
                }
                _ => (),
            }
        }

        Ok(Self {
            name: name.to_string(),
            fonts,
            size,
        })
    }

    /// Get a font by style. If there is no such a font, it will return the REGULAR font.
    pub fn get_by_style(&self, style: FontStyle) -> &Font {
        self.fonts
            .get(&style)
            .unwrap_or_else(|| self.fonts.get(&Regular).unwrap())
    }

    /// Get the regular font
    pub fn get_regular(&self) -> &Font {
        self.fonts.get(&Regular).unwrap()
    }

    /// Get the height of the font
    pub fn get_font_height(&self) -> u32 {
        let font = self.get_regular();
        let metrics = font.metrics();
        ((metrics.ascent - metrics.descent) / metrics.units_per_em as f32 * self.size).ceil() as u32
    }
}

/// A collection of font
///
/// It can be used to draw text on the image.
#[derive(Clone, Debug)]
pub struct FontCollection(Vec<ImageFont>);

unsafe impl Sync for FontCollection {}
unsafe impl Send for FontCollection {}

impl Default for FontCollection {
    fn default() -> Self {
        Self(vec![ImageFont::default()])
    }
}

impl FontCollection {
    /// Create a FontCollection with several fonts.
    pub fn new<S: AsRef<str>>(font_list: &[(S, f32)]) -> Result<Self, FontError> {
        let mut fonts = vec![];
        for (name, size) in font_list {
            let name = name.as_ref();
            match ImageFont::new(name, *size) {
                Ok(font) => fonts.push(font),
                Err(err) => eprintln!("[error] Error occurs when load font `{}`: {}", name, err),
            }
        }
        Ok(Self(fonts))
    }

    fn glyph_for_char(&self, c: char, style: FontStyle) -> Option<(u32, &ImageFont, &Font)> {
        for font in &self.0 {
            let result = font.get_by_style(style);
            if let Some(id) = result.glyph_for_char(c) {
                return Some((id, font, result));
            }
        }
        eprintln!("[warning] No font found for character `{}`", c);
        None
    }

    /// get max height of all the fonts
    pub fn get_font_height(&self) -> u32 {
        self.0
            .iter()
            .map(|font| font.get_font_height())
            .max()
            .unwrap()
    }

    fn layout(&self, text: &str, style: FontStyle) -> (Vec<PositionedGlyph>, u32) {
        let mut delta_x = 0;
        let height = self.get_font_height();

        let glyphs = text
            .chars()
            .filter_map(|c| {
                self.glyph_for_char(c, style).map(|(id, imfont, font)| {
                    let raster_rect = font
                        .raster_bounds(
                            id,
                            imfont.size,
                            Transform2F::default(),
                            HintingOptions::None,
                            RasterizationOptions::GrayscaleAa,
                        )
                        .unwrap();
                    let position =
                        Vector2I::new(delta_x as i32, height as i32) + raster_rect.origin();
                    delta_x += Self::get_glyph_width(font, id, imfont.size);

                    PositionedGlyph {
                        id,
                        font: font.clone(),
                        size: imfont.size,
                        raster_rect,
                        position,
                    }
                })
            })
            .collect();

        (glyphs, delta_x)
    }

    /// Get the width of the given glyph
    fn get_glyph_width(font: &Font, id: u32, size: f32) -> u32 {
        let metrics = font.metrics();
        let advance = font.advance(id).unwrap();
        (advance / metrics.units_per_em as f32 * size).x().ceil() as u32
    }

    /// Get the width of the given text
    pub fn get_text_len(&self, text: &str) -> u32 {
        self.layout(text, Regular).1
    }

    /// Draw the text to a image
    /// return the width of written text
    pub fn draw_text_mut<I>(
        &self,
        image: &mut I,
        color: I::Pixel,
        x: u32,
        y: u32,
        style: FontStyle,
        text: &str,
    ) -> u32
    where
        I: GenericImage,
        <I::Pixel as Pixel>::Subpixel: ValueInto<f32> + Clamp<f32>,
    {
        let metrics = self.0[0].get_regular().metrics();
        let offset =
            (metrics.descent / metrics.units_per_em as f32 * self.0[0].size).round() as i32;

        let (glyphs, width) = self.layout(text, style);

        for glyph in glyphs {
            glyph.draw(offset, |px, py, v| {
                if v <= std::f32::EPSILON {
                    return;
                }
                let (x, y) = ((px + x as i32) as u32, (py + y as i32) as u32);
                let pixel = image.get_pixel(x, y);
                let weighted_color = weighted_sum(pixel, color, 1.0 - v, v);
                image.put_pixel(x, y, weighted_color);
            })
        }

        width
    }
}

impl<'de> Deserialize<'de> for FontCollection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(parse_font_str(&String::deserialize(deserializer)?).unwrap())
    }
}

impl Serialize for FontCollection {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let font_strings: Vec<String> = self
            .0
            .iter()
            .map(|font| format!("{}={}", font.name, font.size))
            .collect();
        let font_str = font_strings.join(";");
        String::serialize(&font_str, serializer)
    }
}

pub fn parse_font_str(s: &str) -> Result<FontCollection, FontError> {
    let fonts = s
        .split(';')
        .filter(|&f| !f.is_empty())
        .map(|f| {
            let (name, size) = f.split_once('=').unwrap();
            (name.to_owned(), size.parse::<f32>().unwrap_or(26.))
        })
        .collect::<Vec<(String, f32)>>();

    FontCollection::new(&fonts)
}

struct PositionedGlyph {
    id: u32,
    font: Font,
    size: f32,
    position: Vector2I,
    raster_rect: RectI,
}

impl PositionedGlyph {
    fn draw<O: FnMut(i32, i32, f32)>(&self, offset: i32, mut o: O) {
        let mut canvas = Canvas::new(self.raster_rect.size(), Format::A8);

        // don't rasterize whitespace(https://github.com/pcwalton/font-kit/issues/7)
        if canvas.size != Vector2I::new(0, 0) {
            self.font
                .rasterize_glyph(
                    &mut canvas,
                    self.id,
                    self.size,
                    Transform2F::from_translation(-self.raster_rect.origin().to_f32()),
                    HintingOptions::None,
                    RasterizationOptions::GrayscaleAa,
                )
                .unwrap();
        }

        for y in (0..self.raster_rect.height()).rev() {
            let (row_start, row_end) =
                (y as usize * canvas.stride, (y + 1) as usize * canvas.stride);
            let row = &canvas.pixels[row_start..row_end];

            for x in 0..self.raster_rect.width() {
                let val = f32::from(row[x as usize]) / 255.0;
                let px = self.position.x() + x;
                let py = self.position.y() + y + offset;

                o(px, py, val);
            }
        }
    }
}
