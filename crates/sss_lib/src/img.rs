use image::imageops::{horizontal_gradient, resize, vertical_gradient, FilterType};
use image::{Rgba, RgbaImage};

use crate::color::ToRgba;
use crate::components::round_corner;
use crate::error::Background as BackgroundError;
use crate::{DynImageContent, GenerationSettings};

#[derive(Clone, Debug)]
pub enum GradientType {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug)]
pub enum Background {
    Solid(Rgba<u8>),
    Gradient(GradientType, Rgba<u8>, Rgba<u8>),
    Image(RgbaImage),
}

impl Default for Background {
    fn default() -> Self {
        Self::Solid("#323232".to_rgba().unwrap())
    }
}

impl Background {
    pub fn to_image(&self, width: u32, height: u32) -> RgbaImage {
        match self {
            Background::Solid(color) => RgbaImage::from_pixel(width, height, color.to_owned()),
            Background::Image(image) => resize(image, width, height, FilterType::Triangle),
            Background::Gradient(t, start, stop) => {
                let mut img = RgbaImage::new(width, height);
                match t {
                    GradientType::Vertical => vertical_gradient(&mut img, start, stop),
                    GradientType::Horizontal => horizontal_gradient(&mut img, start, stop),
                }
                img
            }
        }
    }
}

pub fn generate_image(
    settings: GenerationSettings,
    content: impl DynImageContent,
) -> image::ImageBuffer<Rgba<u8>, Vec<u8>> {
    let mut inner = content.content();
    let (p_x, p_y) = settings.padding;
    let (w, h) = (inner.width() + (p_x * 2), inner.height() + (p_y * 2));

    let mut img = settings.background.to_image(w, h);

    if let Some(radius) = settings.round_corner {
        round_corner(&mut inner, radius);
    }

    if let Some(shadow) = settings.shadow {
        inner = shadow.apply_to(&inner, p_x, p_y);
        image::imageops::overlay(&mut img, &inner, 0, 0);
    } else {
        image::imageops::overlay(&mut img, &inner, p_x.into(), p_y.into());
    }

    img
}

impl TryFrom<String> for Background {
    type Error = BackgroundError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.contains(';') {
            let mut split = value.splitn(3, ';');
            let o = split.next().unwrap();
            let start = split.next().unwrap().to_rgba().unwrap();
            let stop = split.next().unwrap().to_rgba().unwrap();
            let orientation = if o == "h" {
                GradientType::Horizontal
            } else {
                GradientType::Vertical
            };
            return Ok(Background::Gradient(orientation, start, stop));
        }
        if let Ok(color) = value.to_rgba() {
            return Ok(Background::Solid(color));
        }
        if let Ok(img) = image::open(value) {
            return Ok(Background::Image(img.to_rgba8()));
        }
        Err(BackgroundError::CannotParse)
    }
}
