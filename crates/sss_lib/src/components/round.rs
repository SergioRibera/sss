use image::imageops::{crop_imm, resize, FilterType};
use image::{DynamicImage, GenericImage, GenericImageView, Rgba, RgbaImage};
use imageproc::drawing::draw_line_segment_mut;

/// Round the corner of the image
pub fn round_corner(image: &mut DynamicImage, radius: u32) {
    // draw a circle with given foreground on given background
    // then split it into four pieces and paste them to the four corner of the image
    //
    // the circle is drawn on a bigger image to avoid the aliasing
    // later it will be scaled to the correct size
    // we add +1 (to the radius) to make sure that there is also space for the border to mitigate artefacts when scaling
    // note that the +1 isn't added to the radius when drawing the circle
    let mut circle =
        RgbaImage::from_pixel((radius + 1) * 4, (radius + 1) * 4, Rgba([255, 255, 255, 0]));

    let width = image.width();
    let height = image.height();

    // use the bottom right pixel to get the color of the foreground
    let foreground = image.get_pixel(width - 1, height - 1);

    draw_filled_circle_mut(
        &mut circle,
        (((radius + 1) * 2) as i32, ((radius + 1) * 2) as i32),
        radius as i32 * 2,
        foreground,
    );

    // scale down the circle to the correct size
    let circle = resize(
        &circle,
        (radius + 1) * 2,
        (radius + 1) * 2,
        FilterType::Triangle,
    );

    // top left
    let part = crop_imm(&circle, 1, 1, radius, radius);
    image.copy_from(&*part, 0, 0).unwrap();

    // top right
    let part = crop_imm(&circle, radius + 1, 1, radius, radius - 1);
    image.copy_from(&*part, width - radius, 0).unwrap();

    // bottom left
    let part = crop_imm(&circle, 1, radius + 1, radius, radius);
    image.copy_from(&*part, 0, height - radius).unwrap();

    // bottom right
    let part = crop_imm(&circle, radius + 1, radius + 1, radius, radius);
    image
        .copy_from(&*part, width - radius, height - radius)
        .unwrap();
}

// `draw_filled_circle_mut` doesn't work well with small radius in imageproc v0.18.0
// it has been fixed but still have to wait for releasing
// issue: https://github.com/image-rs/imageproc/issues/328
// PR: https://github.com/image-rs/imageproc/pull/330
/// Draw as much of a circle, including its contents, as lies inside the image bounds.
pub fn draw_filled_circle_mut<I>(image: &mut I, center: (i32, i32), radius: i32, color: I::Pixel)
where
    I: GenericImage,
    I::Pixel: 'static,
{
    let mut x = 0i32;
    let mut y = radius;
    let mut p = 1 - radius;
    let x0 = center.0;
    let y0 = center.1;

    while x <= y {
        draw_line_segment_mut(
            image,
            ((x0 - x) as f32, (y0 + y) as f32),
            ((x0 + x) as f32, (y0 + y) as f32),
            color,
        );
        draw_line_segment_mut(
            image,
            ((x0 - y) as f32, (y0 + x) as f32),
            ((x0 + y) as f32, (y0 + x) as f32),
            color,
        );
        draw_line_segment_mut(
            image,
            ((x0 - x) as f32, (y0 - y) as f32),
            ((x0 + x) as f32, (y0 - y) as f32),
            color,
        );
        draw_line_segment_mut(
            image,
            ((x0 - y) as f32, (y0 - x) as f32),
            ((x0 + y) as f32, (y0 - x) as f32),
            color,
        );

        x += 1;
        if p < 0 {
            p += 2 * x + 1;
        } else {
            y -= 1;
            p += 2 * (x - y) + 1;
        }
    }
}
