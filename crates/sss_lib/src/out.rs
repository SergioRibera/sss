use image::codecs::png::PngEncoder;
use image::{ImageBuffer, ImageEncoder, Rgba};
use notify_rust::{Image, Notification};

use crate::str_to_format;

pub fn make_output(
    img: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    output: &str,
    show_notify: bool,
    fmt: Option<&str>,
) {
    match output {
        "raw" => {
            let mut stdout = std::io::stdout();
            let encoder = PngEncoder::new(&mut stdout);
            encoder
                .write_image(img, img.width(), img.height(), image::ColorType::Rgba8)
                .unwrap();
        }
        _ => {
            img.save_with_format(&output, str_to_format(fmt.unwrap_or("png")).unwrap())
                .unwrap();

            if show_notify {
                Notification::new()
                    .summary("Image generated")
                    .body(&format!("Image stored in {output}"))
                    .image_data(
                        Image::from_rgba(img.width() as i32, img.height() as i32, img.to_vec())
                            .unwrap(),
                    )
                    .show()
                    .unwrap();
            }
        }
    }
}
