use std::env::var_os;

use libwayshot::output::OutputPositioning;
use libwayshot::reexport::Transform;
use libwayshot::{CaptureRegion, WayshotConnection};
use screenshots::display_info::DisplayInfo;
use screenshots::image::imageops::{overlay, rotate180, rotate270, rotate90};
use screenshots::image::{Rgba, RgbaImage};
use screenshots::Screen;

type ScreenImage = ((i32, i32, u32, u32, Transform), RgbaImage);

fn wayland_detect() -> bool {
    let xdg_session_type = var_os("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let wayland_display = var_os("WAYLAND_DISPLAY")
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    xdg_session_type.eq("wayland") || wayland_display.to_lowercase().contains("wayland")
}

fn make_all_screens(screens: &[ScreenImage]) -> RgbaImage {
    let max_w = screens.iter().map(|((_, _, w, _, _), _)| *w).sum();
    let max_h = screens
        .iter()
        .map(|((_, _, _, h, _), _)| *h)
        .max()
        .unwrap_or_default();
    let mut res = RgbaImage::from_pixel(max_w, max_h, Rgba([0, 0, 0, 255]));

    for ((x, y, _, _, t), screen_img) in screens {
        let mut img = screen_img.clone();
        match t {
            Transform::_90 => img = rotate90(&img),
            Transform::_180 => img = rotate180(&img),
            Transform::_270 => img = rotate270(&img),
            _ => (),
        }
        overlay(&mut res, &img, (*x).into(), (*y).into());
    }

    res
}

pub struct ShotImpl {
    xorg: Option<Vec<Screen>>,
    wayland: Option<WayshotConnection>,
}

impl Default for ShotImpl {
    fn default() -> Self {
        Self {
            xorg: (!wayland_detect()).then_some(Screen::all().unwrap()),
            wayland: wayland_detect()
                .then_some(WayshotConnection::new())
                .map(|w| w.unwrap()),
        }
    }
}

impl ShotImpl {
    pub fn all(&self, mouse: bool) -> Result<RgbaImage, String> {
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
                        (
                            (x, y, width, height, Transform::Normal),
                            s.capture().unwrap(),
                        )
                    })
                    .collect::<Vec<(_, _)>>(),
            ));
        }

        self.wayland
            .as_ref()
            .ok_or("No Context loaded".to_string())
            .map(|wayshot| {
                let outputs = wayshot.get_all_outputs();
                make_all_screens(
                    &outputs
                        .iter()
                        .map(|o| {
                            let OutputPositioning {
                                x,
                                y,
                                width,
                                height,
                            } = o.dimensions;
                            (
                                (x, y, width as u32, height as u32, o.transform),
                                wayshot
                                    .screenshot_single_output(o, mouse)
                                    .map_err(|_| "Cannot take screenshot on Wayland".to_string())
                                    .unwrap(),
                            )
                        })
                        .collect::<Vec<(_, _)>>(),
                )
            })
    }

    pub fn capture_area(
        &self,
        (x, y, w, h): (i32, i32, u32, u32),
        mouse: bool,
    ) -> Result<RgbaImage, String> {
        if w <= 1 || h <= 1 {
            return Err("The area size is invalid".to_string());
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
                .unwrap();
            return Ok(screen
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
                .unwrap());
        }

        self.wayland
            .as_ref()
            .ok_or("No Context loaded".to_string())
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
                    .map_err(|_| "Cannot take screenshot on Wayland".to_string())
                    .unwrap()
            })
    }
}
