use color_eyre::eyre::Report;
use config::get_config;
use img::Screenshot;
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

fn main() -> Result<(), Report> {
    let (config, g_config) = get_config();

    Ok(generate_image(g_config, Screenshot { config })?)
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
