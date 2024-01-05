use image::{DynamicImage, GenericImage, GenericImageView, Rgba};

/// Round the corner of the image
pub fn round_corner(img: &mut DynamicImage, radius: u32) {
    let (width, height) = img.dimensions();

    // top left
    border_radius(img, radius, |x, y| (x - 1, y - 1));
    // top right
    border_radius(img, radius, |x, y| (width - x, y - 1));
    // bottom right
    border_radius(img, radius, |x, y| (width - x, height - y));
    // bottom left
    border_radius(img, radius, |x, y| (x - 1, height - y));
}

fn border_radius(img: &mut DynamicImage, r: u32, coordinates: impl Fn(u32, u32) -> (u32, u32)) {
    if r == 0 {
        return;
    }
    let r0 = r;

    // 16x antialiasing: 16x16 grid creates 256 possible shades, great for u8!
    let r = 16 * r;

    let mut x = 0;
    let mut y = r - 1;
    let mut p: i32 = 2 - r as i32;

    // ...

    let mut alpha: u16 = 0;
    let mut skip_draw = true;

    let set_alpha = |img: &mut DynamicImage, alpha, (x, y)| {
        let p = img.get_pixel(x, y).0;
        img.put_pixel(x, y, Rgba([p[0], p[1], p[2], alpha]));
    };
    let draw = |img: &mut DynamicImage, alpha, x, y| {
        debug_assert!((1..=256).contains(&alpha));
        let pixel_alpha = img.get_pixel(x, y).0[3];
        set_alpha(
            img,
            ((alpha * pixel_alpha as u16 + 128) / 256) as u8,
            (r0 - x, r0 - y),
        );
    };

    'l: loop {
        // (comments for bottom_right case:)
        // remove contents below current position
        {
            let i = x / 16;
            for j in y / 16 + 1..r0 {
                set_alpha(img, 0, coordinates(r0 - i, r0 - j))
            }
        }
        // remove contents right of current position mirrored
        {
            let j = x / 16;
            for i in y / 16 + 1..r0 {
                set_alpha(img, 0, coordinates(r0 - i, r0 - j))
            }
        }

        // draw when moving to next pixel in x-direction
        if !skip_draw {
            draw(img, alpha, x / 16 - 1, y / 16);
            draw(img, alpha, y / 16, x / 16 - 1);
            alpha = 0;
        }

        for _ in 0..16 {
            skip_draw = false;

            if x >= y {
                break 'l;
            }

            alpha += y as u16 % 16 + 1;
            if p < 0 {
                x += 1;
                p += (2 * x + 2) as i32;
            } else {
                // draw when moving to next pixel in y-direction
                if y % 16 == 0 {
                    draw(img, alpha, x / 16, y / 16);
                    draw(img, alpha, y / 16, x / 16);
                    skip_draw = true;
                    alpha = (x + 1) as u16 % 16 * 16;
                }

                x += 1;
                p -= (2 * (y - x) + 2) as i32;
                y -= 1;
            }
        }
    }

    // one corner pixel left
    if x / 16 == y / 16 {
        // column under current position possibly not yet accounted
        if x == y {
            alpha += y as u16 % 16 + 1;
        }
        let s = y as u16 % 16 + 1;
        let alpha = 2 * alpha - s * s;
        draw(img, alpha, x / 16, y / 16);
    }

    // remove remaining square of content in the corner
    let range = y / 16 + 1..r0;
    for i in range.clone() {
        for j in range.clone() {
            set_alpha(img, 0, coordinates(r0 - i, r0 - j))
        }
    }
}
