use image::imageops::{resize, FilterType};
use image::{Rgba, RgbaImage};

use crate::color::ToRgba;
use crate::components::round_corner;
// use crate::utils::copy_alpha;
use crate::{DynImageContent, GenerationSettings};

#[derive(Clone, Debug)]
pub enum Background {
    Solid(Rgba<u8>),
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
        }
    }
}

pub fn generate_image(
    settings: GenerationSettings,
    content: impl DynImageContent,
) -> image::ImageBuffer<Rgba<u8>, Vec<u8>> {
    let mut inner = content.content();
    let (p_x, p_y) = settings.padding;
    let (w, h) = (
        inner.width() + (p_x * 2),
        inner.height() + (p_y * 2),
    );

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
