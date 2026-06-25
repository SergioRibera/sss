#[cfg(target_os = "linux")]
use arboard::SetExtLinux;
use image::imageops::{horizontal_gradient, resize, vertical_gradient, FilterType};
use image::{Rgba, RgbaImage};
use std::io::Cursor;

use crate::components::{add_window_controls, add_window_title, round_corner};
use crate::error::{Background as BackgroundError, ImagenGeneration};
use crate::font::FontStyle;
use crate::out::make_output;
use crate::ToRgba;
use crate::{DynImageContent, GenerationSettings};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GradientType {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Background {
    Solid(Rgba<u8>),
    Gradient(GradientType, Rgba<u8>, Rgba<u8>),
    Image(RgbaImage),
}

impl Default for Background {
    fn default() -> Self {
        Self::Solid(Rgba([50, 50, 50, 255])) // #323232
    }
}

impl Background {
    pub fn to_image(&self, width: u32, height: u32) -> RgbaImage {
        match self {
            Background::Solid(color) => RgbaImage::from_pixel(width, height, color.to_owned()),
            Background::Image(image) => resize(image, width, height, FilterType::Triangle),
            Background::Gradient(t, start, stop) => {
                let mut img = RgbaImage::new(width, height);
                match t {
                    GradientType::Vertical => vertical_gradient(&mut img, start, stop),
                    GradientType::Horizontal => horizontal_gradient(&mut img, start, stop),
                }
                img
            }
        }
    }
}

pub fn generate_image(
    settings: GenerationSettings,
    content: impl DynImageContent,
) -> Result<(), ImagenGeneration> {
    let mut inner = content.content()?;
    let show_winbar = settings.window_controls.enable || settings.window_controls.title.is_some();
    let (p_x, p_y) = if settings.border {
        settings.padding
    } else {
        // Border off: no padding around the inner image. Background and
        // shadow are also skipped further down (`settings.border` gates
        // those branches), so the output is just the captured frame plus
        // the optional winbar / author footer.
        (0, 0)
    };
    tracing::info!("Padding: ({p_x}, {p_y})");
    let win_bar_h = if show_winbar {
        settings.window_controls.height
    } else {
        0
    };
    let (w, h) = (
        inner.width() + (p_x * 2),
        inner.height() + (p_y * 2) + win_bar_h,
    );

    tracing::info!("Total size: ({w}, {h})");

    let mut winbar = settings
        .colors
        .windows_background
        .to_image(inner.width(), settings.window_controls.height);
    // With the border on, paint the configured background under the
    // padded image. With it off, leave the canvas transparent so the
    // final overlay below just copies the inner image verbatim.
    let mut img = if settings.border {
        settings.colors.background.to_image(w, h)
    } else {
        RgbaImage::new(w, h)
    };

    if settings.window_controls.enable {
        add_window_controls(
            &mut winbar,
            settings.colors.windows_background,
            settings.window_controls.width,
            settings.window_controls.height,
            settings.window_controls.title_padding,
            settings.window_controls.width / 3 / 4,
        )?;
    }

    if let Some(title) = settings.window_controls.title.as_ref() {
        add_window_title(
            &mut winbar,
            &settings.fonts,
            settings.colors.windows_title,
            title,
            settings.window_controls.title_padding,
            settings.window_controls.enable,
            settings.window_controls.width,
            settings.window_controls.height,
        )?;
    }

    if show_winbar {
        let mut tmp_inner = RgbaImage::new(inner.width(), inner.height() + win_bar_h);
        image::imageops::overlay(&mut tmp_inner, &winbar, 0, 0);
        image::imageops::overlay(&mut tmp_inner, &inner, 0, win_bar_h.into());
        inner = tmp_inner;
    }

    if let Some(radius) = settings.round_corner {
        round_corner(&mut inner, radius);
    }

    // Shadow is part of the decorative border — without padding around
    // the inner image it would just smear the screenshot. Skip it when
    // the border is off.
    if let (Some(shadow), true) = (settings.shadow.as_ref(), settings.border) {
        inner = shadow.apply_to(&inner, p_x, p_y);
        image::imageops::overlay(&mut img, &inner, 0, 0);
    } else {
        image::imageops::overlay(&mut img, &inner, p_x.into(), p_y.into());
    }

    // Author footer sits in the bottom padding strip — without padding
    // there's nowhere to draw it without scribbling over the screenshot.
    if let (Some(author), true) = (settings.author.as_ref(), settings.border) {
        let title_w = settings.fonts.get_text_len(author)?;

        settings.fonts.draw_text_mut(
            &mut img,
            settings.colors.author_color,
            w / 2 - title_w / 2,
            h - p_y / 2,
            FontStyle::Bold,
            author,
        )?;
    }

    if settings.copy {
        copy_image_to_clipboard(&img)?;
    }

    // Empty output = caller signalled "don't save". The args layer leaves
    // it empty when `--copy` is on without `--output`; honour that here
    // so we don't drop an unsolicited `out.png` next to the user.
    if settings.output.is_empty() {
        return Ok(());
    }

    make_output(
        &img,
        &settings.output,
        settings.show_notify,
        settings.save_format.as_deref(),
    )
}

impl TryFrom<String> for Background {
    type Error = BackgroundError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.contains(';') {
            let mut split = value.splitn(3, ';');
            let o = split.next().ok_or(BackgroundError::CannotParse)?;
            let start = split
                .next()
                .ok_or(BackgroundError::CannotParse)?
                .to_rgba()?;
            let stop = split
                .next()
                .ok_or(BackgroundError::CannotParse)?
                .to_rgba()?;
            let orientation = if o == "h" {
                GradientType::Horizontal
            } else {
                GradientType::Vertical
            };
            return Ok(Background::Gradient(orientation, start, stop));
        }
        if let Ok(color) = value.to_rgba() {
            return Ok(Background::Solid(color));
        }
        if let Ok(img) = image::open(value) {
            return Ok(Background::Image(img.to_rgba8()));
        }
        Err(BackgroundError::CannotParse)
    }
}

/// Push `img` to the system clipboard as a PNG.
///
/// On Linux Wayland we go through `zwlr_data_control_manager_v1` directly
/// (see [`crate::clipboard`]) so the call returns as soon as a clipboard
/// manager has read the data — no fork-daemon left behind. The fallback
/// is `arboard`, which keeps the surface alive itself via its own
/// `SetExtLinux::wait()` fork (the historical behaviour). Non-Linux
/// platforms always use `arboard`.
fn copy_image_to_clipboard(img: &RgbaImage) -> Result<(), ImagenGeneration> {
    #[cfg(target_os = "linux")]
    {
        // Try the wlroots data-control protocol first.
        let mut png = Vec::with_capacity(img.as_raw().len() / 3);
        image::write_buffer_with_format(
            &mut Cursor::new(&mut png),
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .map_err(|e| ImagenGeneration::Custom(format!("png encode: {e}")))?;

        match crate::clipboard::copy_png(png) {
            Ok(()) => return Ok(()),
            Err(crate::clipboard::WlClipboardError::NotOnWayland) => {
                tracing::debug!("not on wayland; using arboard");
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "native wayland clipboard unavailable; falling back to arboard"
                );
            }
        }
    }

    let mut c = arboard::Clipboard::new()?;
    #[cfg(target_os = "linux")]
    let set = c
        .set()
        .clipboard(arboard::LinuxClipboardKind::Clipboard)
        .wait();
    #[cfg(not(target_os = "linux"))]
    let set = c.set();
    set.image(arboard::ImageData {
        width: img.width() as usize,
        height: img.height() as usize,
        bytes: img.to_vec().into(),
    })?;
    Ok(())
}

/// Push `text` to the system clipboard.
///
/// On Wayland we use the same `zwlr_data_control_manager_v1` path as
/// [`copy_image_to_clipboard`]: hand the bytes to the compositor, wait
/// for the clipboard manager to take over, and return — so the caller
/// can exit cleanly without leaving an arboard fork behind that would
/// die with the process and wipe the selection.
pub fn copy_text_to_clipboard(text: &str) -> Result<(), ImagenGeneration> {
    #[cfg(target_os = "linux")]
    {
        match crate::clipboard::copy_text(text.to_owned()) {
            Ok(()) => return Ok(()),
            Err(crate::clipboard::WlClipboardError::NotOnWayland) => {
                tracing::debug!("not on wayland; using arboard");
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "native wayland clipboard unavailable; falling back to arboard"
                );
            }
        }
    }

    let mut c = arboard::Clipboard::new()?;
    #[cfg(target_os = "linux")]
    let set = c
        .set()
        .clipboard(arboard::LinuxClipboardKind::Clipboard)
        .wait();
    #[cfg(not(target_os = "linux"))]
    let set = c.set();
    set.text(text.to_owned())?;
    Ok(())
}
