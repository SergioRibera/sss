use image::codecs::png::PngEncoder;
use image::{ImageBuffer, ImageEncoder, Rgba};

use crate::str_to_format;

pub fn make_output(img: &ImageBuffer<Rgba<u8>, Vec<u8>>, output: &str, fmt: Option<&str>) {
    match output {
        "raw" => {
            let mut stdout = std::io::stdout();
            let encoder = PngEncoder::new(&mut stdout);
            encoder
                .write_image(
                    &img.to_vec(),
                    img.width(),
                    img.height(),
                    image::ColorType::Rgba8,
                )
                .unwrap();
        }
        _ => {
            img.save_with_format(&output, str_to_format(fmt.unwrap_or("png")).unwrap())
                .unwrap();
        }
    }
}
