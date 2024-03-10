use image::imageops::{resize, FilterType};
use image::{Pixel, Rgba, RgbaImage};
use imageproc::drawing::draw_filled_circle_mut;

/// Round the corner of the image
pub fn round_corner(img: &mut RgbaImage, radius: u32) {
    let (width, height) = img.dimensions();

    let mut circle =
        RgbaImage::from_pixel((radius + 1) * 4, (radius + 1) * 4, Rgba([255, 255, 255, 0]));

    draw_filled_circle_mut(
        &mut circle,
        (((radius + 1) * 2) as i32, ((radius + 1) * 2) as i32),
        radius as i32 * 2,
        Rgba([255, 255, 255, 255]),
    );
    let circle = resize(
        &circle,
        (radius + 1) * 2,
        (radius + 1) * 2,
        FilterType::Triangle,
    );

    let (c_w, c_h) = circle.dimensions();

    for x in 0..radius {
        for y in 0..radius {
            // top left
            let tl_a = circle.get_pixel(x, y).0[3];
            img.get_pixel_mut(x, y).apply_with_alpha(|p| p, |_| tl_a);
            // top right
            let tr_a = circle.get_pixel(c_w - x - 1, y).0[3];
            img.get_pixel_mut(width - x - 1, y)
                .apply_with_alpha(|p| p, |_| tr_a);
            // bottom left
            let bl_a = circle.get_pixel(x, c_h - y - 1).0[3];
            img.get_pixel_mut(x, height - y - 1)
                .apply_with_alpha(|p| p, |_| bl_a);
            // bottom right
            let br_a = circle.get_pixel(c_w - x - 1, c_h - y - 1).0[3];
            img.get_pixel_mut(width - x - 1, height - y - 1)
                .apply_with_alpha(|p| p, |_| br_a);
        }
    }
}
