use image::imageops::{resize, FilterType};
use image::{Rgba, RgbaImage};
use imageproc::drawing::draw_filled_circle_mut;

pub fn round_corner(img: &mut RgbaImage, radius: u32) {
    let (width, height) = img.dimensions();

    let scale = 3;
    let scaled_radius = radius * scale;

    let mut circle = RgbaImage::from_pixel(
        scaled_radius * 2,
        scaled_radius * 2,
        Rgba([255, 255, 255, 0]),
    );

    draw_filled_circle_mut(
        &mut circle,
        (scaled_radius as i32, scaled_radius as i32),
        scaled_radius as i32,
        Rgba([255, 255, 255, 255]),
    );

    let circle = resize(&circle, radius * 2, radius * 2, FilterType::Gaussian);

    let (c_w, c_h) = circle.dimensions();

    for x in 0..radius {
        for y in 0..radius {
            let tl_a = circle.get_pixel(x, y).0[3];
            let tr_a = circle.get_pixel(c_w - x - 1, y).0[3];
            let bl_a = circle.get_pixel(x, c_h - y - 1).0[3];
            let br_a = circle.get_pixel(c_w - x - 1, c_h - y - 1).0[3];

            // Top left
            img.get_pixel_mut(x, y).0[3] = tl_a;
            // Top right
            img.get_pixel_mut(width - x - 1, y).0[3] = tr_a;
            // Bottom left
            img.get_pixel_mut(x, height - y - 1).0[3] = bl_a;
            // Bottom right
            img.get_pixel_mut(width - x - 1, height - y - 1).0[3] = br_a;
        }
    }
}
