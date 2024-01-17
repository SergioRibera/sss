use config::get_config;
use img::Screenshot;
use screenshots::image::error::{ImageFormatHint, UnsupportedError, UnsupportedErrorKind};
use screenshots::image::{ImageError, ImageFormat};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sss_lib::generate_image;

mod config;
mod img;
mod shot;

#[derive(Clone, Copy, Debug, Default)]
pub struct Area {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

fn main() {
    let config = get_config();

    let img = generate_image(
        config.copy,
        config.clone().into(),
        Screenshot {
            config: config.clone(),
        },
    );

    img.save_with_format(
        &config.output,
        str_to_format(config.save_format.unwrap_or("png".to_string())).unwrap(),
    )
    .unwrap();
    println!("Saved!");
}

fn str_to_format(s: String) -> Result<ImageFormat, ImageError> {
    ImageFormat::from_extension(s.clone()).ok_or(ImageError::Unsupported(
        UnsupportedError::from_format_and_kind(
            ImageFormatHint::Name(s.to_string()),
            UnsupportedErrorKind::Format(ImageFormatHint::Name(s.to_string())),
        ),
    ))
}

fn str_to_area(s: &str) -> Result<Area, String> {
    let err = "The format of area is wrong (x,y WxH)".to_string();
    let (pos, size) = s.split_once(' ').ok_or(err.clone())?;
    let (x, y) = pos.split_once(',').ok_or(err.clone()).map(|(x, y)| {
        (
            x.parse::<i32>().map_err(|e| e.to_string()),
            y.parse::<i32>().map_err(|e| e.to_string()),
        )
    })?;
    let (w, h) = size.split_once('x').ok_or(err.clone()).map(|(w, h)| {
        (
            w.parse::<u32>().map_err(|e| e.to_string()),
            h.parse::<u32>().map_err(|e| e.to_string()),
        )
    })?;

    Ok(Area {
        x: x?,
        y: y?,
        width: w?,
        height: h?,
    })
}

impl<'de> Deserialize<'de> for Area {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(str_to_area(&String::deserialize(deserializer)?).unwrap())
    }
}

impl Serialize for Area {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let Area {
            x,
            y,
            width,
            height,
        } = self;
        String::serialize(&format!("{x},{y} {width}x{height}"), serializer)
    }
}
