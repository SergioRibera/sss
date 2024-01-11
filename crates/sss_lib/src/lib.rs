//! This library is originally inspired from https://github.com/Aloxaf/silicon
use font::FontCollection;

pub mod blur;
mod color;
pub mod components;
pub mod error;
pub mod font;
mod img;
mod shadow;
pub mod utils;

pub use color::ToRgba;
pub use image;
use image::{Rgba, RgbaImage};
pub use imageproc;
pub use img::*;
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
    /// Collection of fonts
    /// Default: Default::default()
    pub fonts: FontCollection,
    /// Show author name
    /// Default: None
    pub author: Option<String>,
    /// Author color
    /// Default: #FFFFFF
    pub author_color: Rgba<u8>,
    /// Author font to use
    /// Default: Hack
    pub author_font: String,
    /// Enable Window Controls
    /// Default: false
    pub window_controls: bool,
    /// Window title bar background color
    /// Default: #4287f5
    pub windows_background: Background,
    /// Enable Window Controls
    /// Default: None
    pub windows_title: Option<String>,
    /// Title color
    /// Default: #FFFFFF
    pub windows_title_color: Rgba<u8>,
    /// Width of window controls
    /// Default: 120
    pub window_controls_width: u32,
    /// Height of window controls
    /// Default: 40
    pub window_controls_height: u32,
    /// Title bar padding on horizontal
    /// Default: 10
    pub titlebar_padding: u32,
}

impl Default for GenerationSettings {
    fn default() -> Self {
        Self {
            background: Background::Solid(image::Rgba([0x32, 0x32, 0x32, 255])),
            fonts: FontCollection::default(),
            padding: (80, 100),
            round_corner: Some(15),
            shadow: None,
            author: None,
            author_font: "Hack".to_string(),
            author_color: Rgba([255, 255, 255, 255]),
            window_controls: false,
            windows_background: Background::Solid(Rgba([0x42, 0x87, 0xf5, 255])),
            windows_title: None,
            windows_title_color: Rgba([255, 255, 255, 255]),
            window_controls_width: 120,
            window_controls_height: 40,
            titlebar_padding: 10,
        }
    }
}

pub trait DynImageContent {
    fn content(&self) -> RgbaImage;
}
