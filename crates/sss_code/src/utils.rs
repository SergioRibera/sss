use sss_lib::image::imageops::{resize, FilterType};
use sss_lib::image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
use sss_lib::imageproc::drawing::draw_filled_circle_mut;
use sss_lib::utils::copy_alpha;
use sss_lib::ToRgba;
use syntect::highlighting::Color;

pub fn color_to_rgba(c: Color) -> Rgba<u8> {
    Rgba([c.r, c.g, c.b, c.a])
}

pub fn add_window_controls(
    image: &mut DynamicImage,
    width: u32,
    height: u32,
    padding: u32,
    radius: u32,
) {
    let color = [
        ("#FF5F56", "#E0443E"),
        ("#FFBD2E", "#DEA123"),
        ("#27C93F", "#1AAB29"),
    ];

    let mut background = image.get_pixel(37, 37);
    background.0[3] = 0;

    let mut title_bar = RgbaImage::from_pixel(width * 3, height * 3, background);
    let step = (radius * 2) as i32;
    let spacer = step * 2;
    let center_y = (height / 2) as i32;

    for (i, (fill, outline)) in color.iter().enumerate() {
        draw_filled_circle_mut(
            &mut title_bar,
            ((i as i32 * spacer + step) * 3, center_y * 3),
            (radius + 1) as i32 * 3,
            outline.to_rgba().unwrap(),
        );
        draw_filled_circle_mut(
            &mut title_bar,
            ((i as i32 * spacer + step) * 3, center_y * 3),
            radius as i32 * 3,
            fill.to_rgba().unwrap(),
        );
    }
    // create a big image and resize it to blur the edge
    // it looks better than `blur()`
    let title_bar = resize(&title_bar, width, height, FilterType::Triangle);

    copy_alpha(&title_bar, image.as_mut_rgba8().unwrap(), padding, padding);
}
