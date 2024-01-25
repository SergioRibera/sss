//! This library is originally inspired from https://github.com/Aloxaf/silicon
use font::FontCollection;

mod args;
pub mod blur;
mod color;
pub mod components;
pub mod error;
pub mod font;
mod img;
mod shadow;
pub mod utils;

pub use args::*;
pub use color::ToRgba;
pub use image;
use image::{Rgba, RgbaImage};
pub use imageproc;
pub use img::*;
pub use shadow::Shadow;

#[derive(Clone, Debug)]
pub struct GenerationSettings {
    /// Copy to clipboard
    /// Default: false
    pub copy: bool,
    /// Output img
    /// Not Default
    pub output: String,
    /// Save Format image
    /// Default: png
    pub save_format: Option<String>,
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
    /// Author font to use
    /// Default: Hack
    pub author_font: String,
    /// Set Colors
    pub colors: Colors,
    /// Set Window Controls
    pub window_controls: WindowControls,
}

#[derive(Clone, Debug)]
pub struct Colors {
    /// Author color
    /// Default: #FFFFFF
    pub author_color: Rgba<u8>,
    /// Background for image
    /// Default: #323232
    pub background: Background,
    /// Window title bar background color
    /// Default: #4287f5
    pub windows_background: Background,
    /// Title color
    /// Default: #FFFFFF
    pub windows_title: Rgba<u8>,
}

#[derive(Clone, Debug)]
pub struct WindowControls {
    /// Enable Window Controls
    /// Default: false
    pub enable: bool,
    /// Enable Window Controls
    /// Default: None
    pub title: Option<String>,
    /// Width of window controls
    /// Default: 120
    pub width: u32,
    /// Height of window controls
    /// Default: 40
    pub height: u32,
    /// Title bar padding on horizontal
    /// Default: 10
    pub title_padding: u32,
}

pub trait DynImageContent {
    fn content(&self) -> RgbaImage;
}
