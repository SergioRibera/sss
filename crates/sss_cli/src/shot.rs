//! Thin glue between the `sss` CLI options and the `sss_capture` crate.
//!
//! All platform handling — X11, Wayland (wlr-screencopy + portal), Windows,
//! macOS, multi-monitor composition, rotation, scale-factor handling — lives
//! in `sss_capture`. This file just maps CLI flags into the right
//! `Capturer::capture_*` call.

use sss_capture::{BackendKind, CaptureOptions, Capturer, Point, Rect as CRect};
use sss_lib::error::ImagenGeneration;
use sss_lib::image::RgbaImage;

use crate::error::SSScreenshot;
use crate::Area;

impl From<Area> for CRect {
    fn from(a: Area) -> Self {
        CRect::from_xywh(a.x, a.y, a.width, a.height)
    }
}

pub struct ShotImpl {
    capturer: Capturer,
}

impl ShotImpl {
    pub fn new(show_cursor: bool) -> Result<Self, SSScreenshot> {
        let capturer = Capturer::builder()
            .backend(BackendKind::Auto)
            .options(CaptureOptions {
                show_cursor,
                ..Default::default()
            })
            .build()
            .map_err(|e| SSScreenshot::Custom(e.to_string()))?;
        tracing::info!("sss_capture backend: {}", capturer.backend_name());
        Ok(Self { capturer })
    }

    pub fn all(&self) -> Result<RgbaImage, ImagenGeneration> {
        self.capturer
            .capture_all()
            .map(|i| i.into_rgba())
            .map_err(|e| ImagenGeneration::Custom(e.to_string()))
    }

    pub fn capture_area(&self, area: Area) -> Result<RgbaImage, ImagenGeneration> {
        if area.width <= 1 || area.height <= 1 {
            return Err(ImagenGeneration::Custom(
                "The area size is invalid".to_owned(),
            ));
        }
        self.capturer
            .capture_region(area.into())
            .map(|i| i.into_rgba())
            .map_err(|e| ImagenGeneration::Custom(e.to_string()))
    }

    pub fn screen(
        &self,
        mouse_position: Option<(i32, i32)>,
        id: Option<i32>,
        name: Option<String>,
    ) -> Result<RgbaImage, ImagenGeneration> {
        let monitor = if let Some(id) = id {
            self.capturer
                .monitors()
                .map_err(err)?
                .into_iter()
                .find(|m| m.id().raw() == id as u64)
                .ok_or_else(|| {
                    ImagenGeneration::Custom(format!("monitor with id {id} not found"))
                })?
        } else if let Some(ref n) = name {
            self.capturer.monitor_by_name(n).map_err(err)?
        } else if let Some((x, y)) = mouse_position {
            self.capturer.monitor_at(Point::new(x, y)).map_err(err)?
        } else {
            self.capturer.primary_monitor().map_err(err)?
        };

        self.capturer
            .capture_monitor(&monitor)
            .map(|i| i.into_rgba())
            .map_err(err)
    }

    pub fn window(&self, target: &str) -> Result<RgbaImage, ImagenGeneration> {
        let win = if let Ok(id) = target.parse::<u64>() {
            self.capturer
                .window_by_id(sss_capture::WindowId::new(id))
                .map_err(err)?
        } else {
            self.capturer.window_by_title(target).map_err(err)?
        };
        self.capturer
            .capture_window(&win)
            .map(|i| i.into_rgba())
            .map_err(err)
    }
}

fn err(e: sss_capture::CaptureError) -> ImagenGeneration {
    ImagenGeneration::Custom(e.to_string())
}
