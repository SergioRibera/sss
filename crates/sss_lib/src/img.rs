#[cfg(target_os = "linux")]
use arboard::SetExtLinux;
use image::imageops::{horizontal_gradient, resize, vertical_gradient, FilterType};
use image::{Rgba, RgbaImage};

use crate::color::ToRgba;
use crate::components::{add_window_controls, add_window_title, round_corner};
use crate::error::Background as BackgroundError;
use crate::font::FontStyle;
use crate::out::make_output;
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
        Self::Solid("#323232".to_rgba().unwrap())
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

pub fn generate_image(settings: GenerationSettings, content: impl DynImageContent) {
    let mut inner = content.content();
    let show_winbar = settings.window_controls.enable || settings.window_controls.title.is_some();
    let (p_x, p_y) = settings.padding;
    let win_bar_h = if show_winbar {
        settings.window_controls.height
    } else {
        0
    };
    let (w, h) = (
        inner.width() + (p_x * 2),
        inner.height() + (p_y * 2) + win_bar_h,
    );

    let mut winbar = settings
        .colors
        .windows_background
        .to_image(inner.width(), settings.window_controls.height);
    let mut img = settings.colors.background.to_image(w, h);

    if settings.window_controls.enable {
        add_window_controls(
            &mut winbar,
            settings.colors.windows_background,
            settings.window_controls.width,
            settings.window_controls.height,
            settings.window_controls.title_padding,
            settings.window_controls.width / 3 / 4,
        );
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
        );
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

    if let Some(shadow) = settings.shadow {
        inner = shadow.apply_to(&inner, p_x, p_y);
        image::imageops::overlay(&mut img, &inner, 0, 0);
    } else {
        image::imageops::overlay(&mut img, &inner, p_x.into(), p_y.into());
    }

    if let Some(author) = settings.author {
        let title_w = settings.fonts.get_text_len(&author);

        settings.fonts.draw_text_mut(
            &mut img,
            settings.colors.author_color,
            w / 2 - title_w / 2,
            h - p_y / 2,
            FontStyle::Bold,
            &author,
        );
    }

    if settings.copy {
        let mut c = arboard::Clipboard::new().unwrap();

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
        })
        .unwrap();
    }
    make_output(
        &img,
        &settings.output,
        settings.show_notify,
        settings.save_format.as_deref(),
    );
}

impl TryFrom<String> for Background {
    type Error = BackgroundError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.contains(';') {
            let mut split = value.splitn(3, ';');
            let o = split.next().unwrap();
            let start = split.next().unwrap().to_rgba().unwrap();
            let stop = split.next().unwrap().to_rgba().unwrap();
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
