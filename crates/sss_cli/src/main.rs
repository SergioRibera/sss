use config::get_config;
use img::Screenshot;
use sss_lib::generate_image;

mod config;
mod img;
mod shot;

fn main() {
    let config = get_config();

    let img = generate_image(
        config.just_copy,
        config.clone().into(),
        Screenshot {
            config: config.clone(),
        },
    );

    if let Some(path) = config.output {
        img.save_with_format(path, config.save_format).unwrap();
        println!("Saved!");
    }
}
