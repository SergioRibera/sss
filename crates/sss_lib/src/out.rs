use image::codecs::png::PngEncoder;
use image::{ImageBuffer, ImageEncoder, Rgba};
use notify_rust::Notification;

#[cfg(target_os = "linux")]
use notify_rust::Image;

use crate::{error, str_to_format};

pub fn make_output(
    img: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    output: &str,
    show_notify: bool,
    fmt: Option<&str>,
) -> Result<(), error::ImagenGeneration> {
    tracing::trace!("Making Output");
    match output {
        "raw" => {
            let mut stdout = std::io::stdout();
            let encoder = PngEncoder::new(&mut stdout);
            encoder.write_image(img, img.width(), img.height(), image::ColorType::Rgba8)?;
        }
        _ => {
            let format_img = str_to_format(fmt.unwrap_or("png"))?;
            tracing::debug!("Format Image to save: {format_img:?}");
            img.save_with_format(&output, format_img)?;

            if show_notify {
                tracing::trace!("Show notification");
                #[cfg(all(unix, not(target_os = "macos"), not(target_os = "windows")))]
                Notification::new()
                    .summary("Image generated")
                    .body(&format!("Image stored in {output}"))
                    .image_data(Image::from_rgba(
                        img.width() as i32,
                        img.height() as i32,
                        img.to_vec(),
                    )?)
                    .show()?;
                #[cfg(all(target_os = "macos", target_os = "windows"))]
                Notification::new()
                    .summary("Image generated")
                    .body(&format!("Image stored in {output}"))
                    .show()?;
            }
        }
    }
    tracing::trace!("End Output");
    Ok(())
}
