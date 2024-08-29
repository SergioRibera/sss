use std::env::var_os;

use sss_lib::error::ImagenGeneration;
use xcap::image::imageops::overlay;
#[cfg(target_os = "linux")]
use xcap::image::imageops::{rotate180, rotate270, rotate90};
use xcap::image::{Rgba, RgbaImage};
use xcap::Monitor;

use crate::error::SSScreenshot;
use crate::Area;

type ScreenImage = ((Area, f32), RgbaImage);

#[allow(unused)]
fn wayland_detect() -> bool {
    let xdg_session_type = var_os("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let wayland_display = var_os("WAYLAND_DISPLAY")
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    tracing::info!("XDG SESSION: {xdg_session_type:?} - WAYLAND DISPLAY: {wayland_display:?}");

    xdg_session_type.eq("wayland") || wayland_display.to_lowercase().contains("wayland")
}

#[cfg(target_os = "linux")]
fn rotate(screen: &RgbaImage, t: f32) -> RgbaImage {
    match t {
        90.0 => rotate90(screen),
        180.0 => rotate180(screen),
        270.0 => rotate270(screen),
        _ => screen.clone(),
    }
}

fn make_all_screens(screens: &[ScreenImage]) -> RgbaImage {
    let max_w = screens.iter().map(|(a, _)| a.0.width).sum();
    let max_h = screens
        .iter()
        .map(|(a, _)| a.0.height)
        .max()
        .unwrap_or_default();
    let mut res = RgbaImage::from_pixel(max_w, max_h, Rgba([0, 0, 0, 255]));

    for (a, screen_img) in screens {
        #[cfg(target_os = "linux")]
        let screen_img = &rotate(screen_img, a.1);
        overlay(&mut res, screen_img, (a.0.x).into(), (a.0.y).into());
    }

    res
}

pub struct ShotImpl {
    monitors: Vec<Monitor>,
}

impl ShotImpl {
    pub fn new() -> Result<Self, SSScreenshot> {
        Ok(Self {
            monitors: Monitor::all().map_err(|e| SSScreenshot::Custom(e.to_string()))?,
        })
    }

    pub fn all(&self) -> Result<RgbaImage, ImagenGeneration> {
        Ok(make_all_screens(
            &self
                .monitors
                .iter()
                .map(|s| {
                    let x = s.x();
                    let y = s.y();
                    let width = s.width();
                    let height = s.height();
                    s.capture_image()
                        .map(|c| {
                            (
                                (
                                    Area {
                                        x,
                                        y,
                                        width,
                                        height,
                                    },
                                    s.rotation(),
                                ),
                                c,
                            )
                        })
                        .map_err(|e| ImagenGeneration::Custom(e.to_string()))
                })
                .collect::<Result<Vec<(_, _)>, ImagenGeneration>>()?,
        ))
    }

    pub fn capture_area(
        &self,
        Area {
            x,
            y,
            width: w,
            height: h,
        }: Area,
    ) -> Result<RgbaImage, ImagenGeneration> {
        if w <= 1 || h <= 1 {
            return Err(ImagenGeneration::Custom(
                "The area size is invalid".to_owned(),
            ));
        }
        let screen = self
            .monitors
            .iter()
            .find(|s| {
                x >= s.x()
                    && x < s.x() + s.width() as i32
                    && y >= s.y()
                    && y < s.y() + s.height() as i32
            })
            .ok_or(ImagenGeneration::Custom(format!(
                "Screen not found in area: {x},{y} {w}x{h}"
            )))?;
        return screen
            .capture_image()
            .map(|i| {
                xcap::image::imageops::crop_imm(
                    &i,
                    (x - screen.x()) as u32,
                    (y - screen.y()) as u32,
                    w,
                    h,
                )
                .to_image()
            })
            .map_err(|e| ImagenGeneration::Custom(e.to_string()));
    }

    pub fn screen(
        &self,
        mouse_position: Option<(i32, i32)>,
        id: Option<i32>,
        name: Option<String>,
    ) -> Result<RgbaImage, ImagenGeneration> {
        let (x, y) = mouse_position.unwrap_or_default();

        let screen = self.monitors
                .iter()
                .find(|s| {
                    id.map(|i| i as u32).is_some_and(|id| id == s.id())
                        || x >= s.x()
                            && (x - s.width() as i32)
                                < s.x() + s.width() as i32
                            && y >= s.y()
                            && (y - s.height() as i32)
                                < s.y() + s.height() as i32
                }).ok_or(ImagenGeneration::Custom(format!(
                    "Screen not found in mouse position {mouse_position:?} or with id: {id:?} or with name: {name:?}"
                )))?;
        screen
            .capture_image()
            .map_err(|e| ImagenGeneration::Custom(e.to_string()))
    }
}
