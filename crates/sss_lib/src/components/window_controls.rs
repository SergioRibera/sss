use image::imageops::{resize, FilterType};
use image::{Rgba, RgbaImage};
use imageproc::drawing::draw_filled_circle_mut;

use crate::error::ImagenGeneration;
use crate::font::{FontCollection, FontStyle};
use crate::utils::copy_alpha;
use crate::{Background, ToRgba};

pub fn add_window_controls(
    image: &mut RgbaImage,
    background: Background,
    width: u32,
    height: u32,
    padding: u32,
    radius: u32,
) -> Result<(), ImagenGeneration> {
    let color = [
        ("#FF5F56", "#E0443E"),
        ("#FFBD2E", "#DEA123"),
        ("#27C93F", "#1AAB29"),
    ];

    let mut controls = background.to_image(width * 3, height * 3);
    let step = (radius * 2) as i32;
    let spacer = step * 2;
    let center_y = (height / 2) as i32;

    for (i, (fill, outline)) in color.iter().enumerate() {
        draw_filled_circle_mut(
            &mut controls,
            ((i as i32 * spacer + step) * 3, center_y * 3),
            (radius + 1) as i32 * 3,
            outline.to_rgba()?,
        );
        draw_filled_circle_mut(
            &mut controls,
            ((i as i32 * spacer + step) * 3, center_y * 3),
            radius as i32 * 3,
            fill.to_rgba()?,
        );
    }
    // create a big image and resize it to blur the edge
    // it looks better than `blur()`
    let controls = resize(&controls, width, height, FilterType::Triangle);

    copy_alpha(&controls, image, padding, 0);

    Ok(())
}

pub fn add_window_title(
    image: &mut RgbaImage,
    font: &FontCollection,
    color: Rgba<u8>,
    title: &str,
    title_padding: u32,
    window_controls: bool,
    controls_width: u32,
    controls_height: u32,
) -> Result<(), ImagenGeneration> {
    font.draw_text_mut(
        image,
        color,
        title_padding
            + if window_controls {
                controls_width + title_padding
            } else {
                title_padding
            },
        (controls_height / 2) - font.get_font_height()? / 2,
        FontStyle::Bold,
        title,
    )?;

    Ok(())
}
