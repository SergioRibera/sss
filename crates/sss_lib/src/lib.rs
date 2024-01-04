mod color;
mod image;

pub struct GenerationSettings {
    background: String,
    /// pad between code and edge of code area.
    /// Default: 25
    padding: (u32, u32),
    /// Title bar padding
    /// Default: 15
    title_bar_pad: u32,
    /// round corner
    /// Default: true
    round_corner: bool,
    /// Shadow adder
    shadow_adder: Option<ShadowAdder>,
}

pub trait DynImageContent {
    fn content(&self) -> DynamicImage::ImageRgba8;
}

pub fn generate_image(settings: GenerationSettings, content: impl DynImageContent) {
    let mut img =
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(size.0, size.1, background.to_rgba()));
}
