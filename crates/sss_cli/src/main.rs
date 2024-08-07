use color_eyre::eyre::Report;
use config::get_config;
use img::Screenshot;
use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};
use sss_lib::generate_image;
use tracing_subscriber::EnvFilter;

mod config;
mod error;
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
    // install tracing
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new("off"))?)
        .with_timer(tracing_subscriber::fmt::time::Uptime::default())
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    // install color eyre
    color_eyre::config::HookBuilder::default()
        .issue_url(concat!(env!("CARGO_PKG_REPOSITORY"), "/issues/new"))
        .add_issue_metadata("version", env!("CARGO_PKG_VERSION"))
        .issue_filter(|kind| match kind {
            color_eyre::ErrorKind::NonRecoverable(_) => false,
            color_eyre::ErrorKind::Recoverable(_) => true,
        })
        .install()?;

    let (config, g_config) = get_config()?;
    tracing::trace!("Configs loaded");

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
        str_to_area(&String::deserialize(deserializer)?).map_err(D::Error::custom)
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
