//! This library is originally inspired from https://github.com/Aloxaf/silicon
use ::image::DynamicImage;

pub mod blur;
mod color;
pub mod components;
pub mod error;
mod img;
mod shadow;
pub mod utils;

pub use image;
pub use imageproc;
pub use img::*;
pub use color::ToRgba;
pub use shadow::Shadow;

pub struct GenerationSettings {
    /// Background for image
    /// Default: #323232
    pub background: Background,
    /// pad between inner immage and edge.
    /// Default: 25
    pub padding: (u32, u32),
    /// round corner
    /// Default: Some(15)
    pub round_corner: Option<u32>,
    /// Shadow
    /// Default: None
    pub shadow: Option<Shadow>,
}

impl Default for GenerationSettings {
    fn default() -> Self {
        Self {
            background: Background::Solid(image::Rgba([0x32, 0x32, 0x32, 255])),
            padding: (80, 100),
            round_corner: Some(15),
            shadow: None,
        }
    }
}

pub trait DynImageContent {
    fn content(&self) -> DynamicImage;
}
