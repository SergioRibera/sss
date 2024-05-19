use std::env::var_os;

#[cfg(target_os = "linux")]
use libwayshot::output::OutputPositioning;
#[cfg(target_os = "linux")]
use libwayshot::reexport::Transform;
#[cfg(target_os = "linux")]
use libwayshot::{CaptureRegion, WayshotConnection};
use screenshots::display_info::DisplayInfo;
use screenshots::image::imageops::overlay;
#[cfg(target_os = "linux")]
use screenshots::image::imageops::{rotate180, rotate270, rotate90};
use screenshots::image::{Rgba, RgbaImage};
use screenshots::Screen;
use sss_lib::error::ImagenGeneration;

use crate::error::SSScreenshot;
use crate::Area;

#[cfg(target_os = "linux")]
type ScreenImage = ((Area, Transform), RgbaImage);
#[cfg(not(target_os = "linux"))]
type ScreenImage = ((Area, ()), RgbaImage);

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
fn rotate(screen: &RgbaImage, t: Transform) -> RgbaImage {
    match t {
        Transform::_90 => rotate90(screen),
        Transform::_180 => rotate180(screen),
        Transform::_270 => rotate270(screen),
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
    xorg: Option<Vec<Screen>>,
    #[cfg(target_os = "linux")]
    wayland: Option<WayshotConnection>,
    #[cfg(not(target_os = "linux"))]
    wayland: Option<()>,
}

impl ShotImpl {
    pub fn new() -> Result<Self, SSScreenshot> {
        Ok(Self {
            xorg: (!wayland_detect())
                .then_some(Screen::all().map_err(|e| SSScreenshot::Custom(e.to_string()))?),
            #[cfg(target_os = "linux")]
            wayland: wayland_detect().then_some(WayshotConnection::new()?),
            #[cfg(not(target_os = "linux"))]
            wayland: None,
        })
    }

    pub fn all(&self, mouse: bool) -> Result<RgbaImage, ImagenGeneration> {
        if let Some(screens) = self.xorg.as_ref() {
            return Ok(make_all_screens(
                &screens
                    .iter()
                    .map(|s| {
                        let DisplayInfo {
                            x,
                            y,
                            width,
                            height,
                            ..
                        } = s.display_info;
                        s.capture()
                            .map(|c| {
                                (
                                    #[cfg(target_os = "linux")]
                                    (
                                        Area {
                                            x,
                                            y,
                                            width,
                                            height,
                                        },
                                        Transform::Normal,
                                    ),
                                    #[cfg(not(target_os = "linux"))]
                                    (
                                        Area {
                                            x,
                                            y,
                                            width,
                                            height,
                                        },
                                        (),
                                    ),
                                    c,
                                )
                            })
                            .map_err(|e| ImagenGeneration::Custom(e.to_string()))
                    })
                    .collect::<Result<Vec<(_, _)>, ImagenGeneration>>()?,
            ));
        }

        #[cfg(not(target_os = "linux"))]
        return Err(ImagenGeneration::Custom("No Context loaded".to_string()));

        #[cfg(target_os = "linux")]
        self.wayland
            .as_ref()
            .ok_or(ImagenGeneration::Custom("No Context loaded".to_owned()))
            .map(|wayshot| {
                let outputs = wayshot.get_all_outputs();
                Ok(make_all_screens(
                    &outputs
                        .iter()
                        .map(|o| {
                            let OutputPositioning {
                                x,
                                y,
                                width,
                                height,
                            } = o.dimensions;
                            wayshot
                                .screenshot_single_output(o, mouse)
                                .map_err(|e| ImagenGeneration::Custom(e.to_string()))
                                .map(|c| {
                                    (
                                        (
                                            Area {
                                                x,
                                                y,
                                                width: width as u32,
                                                height: height as u32,
                                            },
                                            o.transform,
                                        ),
                                        c,
                                    )
                                })
                        })
                        .collect::<Result<Vec<(_, _)>, ImagenGeneration>>()?,
                ))
            })?
    }

    pub fn capture_area(
        &self,
        Area {
            x,
            y,
            width: w,
            height: h,
        }: Area,
        mouse: bool,
    ) -> Result<RgbaImage, ImagenGeneration> {
        if w <= 1 || h <= 1 {
            return Err(ImagenGeneration::Custom(
                "The area size is invalid".to_owned(),
            ));
        }
        if let Some(screens) = self.xorg.as_ref() {
            let screen = screens
                .iter()
                .find(|s| {
                    x >= s.display_info.x
                        && (x - s.display_info.width as i32)
                            < s.display_info.x + s.display_info.width as i32
                        && y >= s.display_info.y
                        && (y - s.display_info.height as i32)
                            < s.display_info.y + s.display_info.height as i32
                })
                .ok_or(ImagenGeneration::Custom(format!(
                    "Screen not found in area: {x},{y} {w}x{h}"
                )))?;
            return screen
                .capture_area(
                    if x >= screen.display_info.width as i32 {
                        x - screen.display_info.width as i32
                    } else {
                        x
                    },
                    if y >= screen.display_info.height as i32 {
                        y - screen.display_info.height as i32
                    } else {
                        y
                    },
                    w,
                    h,
                )
                .map_err(|e| ImagenGeneration::Custom(e.to_string()));
        }

        #[cfg(not(target_os = "linux"))]
        return Err(ImagenGeneration::Custom("No Context loaded".to_string()));

        #[cfg(target_os = "linux")]
        self.wayland
            .as_ref()
            .ok_or(ImagenGeneration::Custom("No Context loaded".to_string()))
            .map(|wayshot| {
                wayshot
                    .screenshot(
                        CaptureRegion {
                            x_coordinate: x,
                            y_coordinate: y,
                            width: w as i32,
                            height: h as i32,
                        },
                        mouse,
                    )
                    .map_err(|e| ImagenGeneration::Custom(e.to_string()))
            })?
    }

    pub fn screen(
        &self,
        mouse_position: Option<(i32, i32)>,
        id: Option<i32>,
        name: Option<String>,
        mouse: bool,
    ) -> Result<RgbaImage, ImagenGeneration> {
        let pos = mouse_position.or(id.map(|i| (i, i))).unwrap_or_default();

        if let Some(screens) = self.xorg.as_ref() {
            let (x, y) = pos;
            let screen = screens
                .iter()
                .find(|s| {
                    s.display_info.id == pos.0 as u32
                        || x >= s.display_info.x
                            && (x - s.display_info.width as i32)
                                < s.display_info.x + s.display_info.width as i32
                            && y >= s.display_info.y
                            && (y - s.display_info.height as i32)
                                < s.display_info.y + s.display_info.height as i32
                }).ok_or(ImagenGeneration::Custom(format!(
                    "Screen not found in mouse position {mouse_position:?} or with id: {id:?} or with name: {name:?}"
                )))?;
            return screen
                .capture()
                .map_err(|e| ImagenGeneration::Custom(e.to_string()));
        }

        #[cfg(not(target_os = "linux"))]
        return Err(ImagenGeneration::Custom("No Context loaded".to_string()));

        let screen_name = name.unwrap_or_default();

        #[cfg(target_os = "linux")]
        self.wayland
            .as_ref()
            .ok_or(ImagenGeneration::Custom("No Context loaded".to_string()))
            .map(|wayshot| {
                let outputs = wayshot.get_all_outputs();
                let screen = outputs
                    .iter()
                    .find(|o| {
                        let OutputPositioning {
                            x,
                            y,
                            width,
                            height,
                        } = o.dimensions;
                        o.name == screen_name.trim()
                            || pos.0 >= x
                                && (pos.0 - width) < x + width
                                && pos.1 >= y
                                && (pos.1 - height) < y + height
                    }).ok_or(ImagenGeneration::Custom(format!(
                    "Screen not found in mouse position {mouse_position:?} or with id: {id:?} or with name: {screen_name:?}"
                )))?;
                let img = wayshot
                    .screenshot_single_output(screen, mouse)
                    .map_err(|e| ImagenGeneration::Custom(e.to_string()))?;
                #[cfg(target_os = "linux")]
                Ok(rotate(&img, screen.transform))
            })?
    }
}
