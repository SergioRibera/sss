use image::{DynamicImage, Rgba, RgbaImage};
use imageproc::drawing::draw_filled_rect_mut;
use imageproc::rect::Rect;

use crate::color::ToRgba;
use crate::utils::copy_alpha;
use crate::Background;

/// Add the shadow for image
#[derive(Debug)]
pub struct Shadow {
    pub background: Background,
    pub use_inner_image: bool,
    pub shadow_color: Rgba<u8>,
    pub blur_radius: f32,
}

impl Default for Shadow {
    fn default() -> Self {
        Self {
            background: Background::default(),
            use_inner_image: false,
            shadow_color: "#707070".to_rgba().unwrap(),
            blur_radius: 50.0,
        }
    }
}

impl Shadow {
    pub fn apply_to(&self, image: &DynamicImage, p_x: u32, p_y: u32) -> DynamicImage {
        // the size of the final image
        let width = image.width() + p_x * 2;
        let height = image.height() + p_y * 2;

        // create the shadow
        let mut shadow = self.background.to_image(width, height);
        if self.blur_radius > 0.0 {
            shadow = if self.use_inner_image {
                let img = image.to_rgba8();
                crate::blur::gaussian_blur(img, self.blur_radius)
            } else {
                let rect = Rect::at(p_x as i32, p_y as i32).of_size(image.width(), image.height());
                draw_filled_rect_mut(&mut shadow, rect, self.shadow_color);
                crate::blur::gaussian_blur(shadow, self.blur_radius)
            };
        }

        // copy the original image to the top of it
        copy_alpha(image.as_rgba8().unwrap(), &mut shadow, p_x, p_y);

        DynamicImage::ImageRgba8(shadow)
    }
}
