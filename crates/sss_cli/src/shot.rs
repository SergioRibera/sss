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

#[cfg(target_os = "linux")]
type ScreenImage = ((i32, i32, u32, u32, Transform), RgbaImage);
#[cfg(not(target_os = "linux"))]
type ScreenImage = ((i32, i32, u32, u32), RgbaImage);

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
    let max_w = screens.iter().map(|(a, _)| a.2).sum();
    let max_h = screens.iter().map(|(a, _)| a.3).max().unwrap_or_default();
    let mut res = RgbaImage::from_pixel(max_w, max_h, Rgba([0, 0, 0, 255]));

    for (a, screen_img) in screens {
        #[cfg(target_os = "linux")]
        let screen_img = &rotate(screen_img, a.4);
        overlay(&mut res, screen_img, (a.0).into(), (a.1).into());
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

impl Default for ShotImpl {
    fn default() -> Self {
        Self {
            xorg: (!wayland_detect()).then_some(Screen::all().unwrap()),
            #[cfg(target_os = "linux")]
            wayland: wayland_detect()
                .then_some(WayshotConnection::new())
                .map(|w| w.unwrap()),
            #[cfg(not(target_os = "linux"))]
            wayland: None,
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
                            #[cfg(target_os = "linux")]
                            (x, y, width, height, Transform::Normal),
                            #[cfg(not(target_os = "linux"))]
                            (x, y, width, height),
                            s.capture().unwrap(),
                        )
                    })
                    .collect::<Vec<(_, _)>>(),
            ));
        }

        #[cfg(not(target_os = "linux"))]
        return Err("No Context loaded".to_string());

        #[cfg(target_os = "linux")]
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

        #[cfg(not(target_os = "linux"))]
        return Err("No Context loaded".to_string());

        #[cfg(target_os = "linux")]
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

    pub fn screen(
        &self,
        mouse_position: Option<(i32, i32)>,
        id: Option<i32>,
        name: Option<String>,
        mouse: bool,
    ) -> Result<RgbaImage, String> {
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
                })
                .unwrap();
            return Ok(screen.capture().unwrap());
        }

        #[cfg(not(target_os = "linux"))]
        return Err("No Context loaded".to_string());

        let Some(screen_name) = name else {
            return Err("No name set".to_string());
        };

        #[cfg(target_os = "linux")]
        self.wayland
            .as_ref()
            .ok_or("No Context loaded".to_string())
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
                    })
                    .ok_or(format!("Screen '{screen_name}' not found"))
                    .unwrap();
                let img = wayshot
                    .screenshot_single_output(&screen, mouse)
                    .map_err(|_| "Cannot take screenshot on Wayland".to_string())
                    .unwrap();
                #[cfg(target_os = "linux")]
                rotate(&img, screen.transform)
            })
    }
}
