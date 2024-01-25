use image::imageops::overlay;
use image::RgbaImage;

use crate::utils::copy_alpha;
use crate::Background;

/// Add the shadow for image
#[derive(Clone, Debug)]
pub struct Shadow {
    pub shadow_color: Background,
    pub use_inner_image: bool,
    pub blur_radius: f32,
}

impl Default for Shadow {
    fn default() -> Self {
        Self {
            shadow_color: Background::default(),
            use_inner_image: false,
            blur_radius: 50.0,
        }
    }
}

impl Shadow {
    pub fn apply_to(&self, image: &RgbaImage, p_x: u32, p_y: u32) -> RgbaImage {
        assert!(self.blur_radius > 0.);
        // the size of the final image
        let width = image.width() + p_x * 2;
        let height = image.height() + p_y * 2;

        // create the shadow
        let content = if self.use_inner_image {
            image.clone()
        } else {
            self.shadow_color.to_image(image.width(), image.height())
        };

        let mut shadow = RgbaImage::new(width, height);
        overlay(&mut shadow, &content, p_x.into(), p_y.into());

        shadow = crate::blur::gaussian_blur(shadow.clone(), self.blur_radius);

        // copy the original image to the top of it
        copy_alpha(image, &mut shadow, p_x, p_y);

        shadow
    }
}
