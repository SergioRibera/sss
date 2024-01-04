use sss_lib::{generate_image, DynImageContent, GenerationSettings, Shadow};

mod config;

struct Screenshot;

impl DynImageContent for Screenshot {
    fn content(&self) -> sss_lib::image::DynamicImage {
        let binding = screenshots::Screen::all().unwrap();
        let screens = binding.last().unwrap();
        let img = screens.capture().unwrap();
        sss_lib::image::DynamicImage::ImageRgba8(img)
    }
}

fn main() {
    let img = generate_image(GenerationSettings {
        shadow: Some(Shadow::default()),
        ..Default::default()
    }, Screenshot);

    img.save("./algo.png").unwrap();
}
