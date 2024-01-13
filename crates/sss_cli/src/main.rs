use config::get_config;
use img::Screenshot;
use sss_lib::generate_image;

mod config;
mod img;
mod shot;

fn main() {
    let config = get_config();

    let img = generate_image(
        config.clone().into(),
        Screenshot {
            config: config.clone(),
        },
    );

    // if config.just_copy {
    //     let mut c = arboard::Clipboard::new().unwrap();
    //     c.set_image(arboard::ImageData {
    //         width: img.width() as usize,
    //         height: img.height() as usize,
    //         bytes: std::borrow::Cow::Owned(img.to_vec()),
    //     })
    //     .unwrap();
    //     return;
    // }

    if let Some(path) = config.output {
        img.save_with_format(path, config.save_format).unwrap();
        println!("Saved!");
    }
}
