use config::get_config;
use sss_lib::{generate_image, DynImageContent};

mod config;

struct Screenshot {
    area: sss_select::Area,
}

impl DynImageContent for Screenshot {
    fn content(&self) -> sss_lib::image::DynamicImage {
        let binding = screenshots::Screen::all().unwrap();
        let screens = binding.last().unwrap();
        let img = screens.capture().unwrap();
        sss_lib::image::DynamicImage::ImageRgba8(img)
    }
}

fn main() {
    let config = get_config();
    let area = sss_select::get_area(config.clone().into());

    let img = generate_image(config.clone().into(), Screenshot { area });

    if config.just_copy {
        let mut c = arboard::Clipboard::new().unwrap();
        c.set_image(arboard::ImageData {
            width: img.width() as usize,
            height: img.height() as usize,
            bytes: std::borrow::Cow::Owned(img.to_vec()),
        })
        .unwrap();
        return;
    }

    if let Some(path) = config.save_path {
        img.save_with_format(path, config.save_format).unwrap();
    }
}
