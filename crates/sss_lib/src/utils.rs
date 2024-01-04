use image::{Pixel, RgbaImage};

/// copy from src to dst, taking into account alpha channels
pub(crate) fn copy_alpha(src: &RgbaImage, dst: &mut RgbaImage, x: u32, y: u32) {
    assert!(src.width() + x <= dst.width());
    assert!(src.height() + y <= dst.height());
    for j in 0..src.height() {
        for i in 0..src.width() {
            let s = src.get_pixel(i, j);
            let mut d = dst.get_pixel(i + x, j + y).clone();
            match s.0[3] {
                255 => {
                    d = *s;
                }
                0 => (/* do nothing */),
                _ => {
                    d.blend(s);
                }
            }
            dst.put_pixel(i + x, j + y, d.clone());
        }
    }
}
