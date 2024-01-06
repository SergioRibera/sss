///! This file is inspired from https://github.com/Aloxaf/silicon
///!
///! Source from: https://github.com/Aloxaf/silicon/blob/master/src/formatter.rs
///!
use std::borrow::Cow;
use std::ops::Range;

use sss_lib::image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
use sss_lib::utils::copy_alpha;
use sss_lib::DynImageContent;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Color, Style, Theme};
use syntect::parsing::{SyntaxReference, SyntaxSet};

use crate::config::CodeConfig;
use crate::font::{FontCollection, FontStyle};
use crate::utils::{add_window_controls, color_to_rgba};

type Drawable = (u32, u32, Option<Color>, FontStyle, String);

const LINE_SPACE: u32 = 5;
const LINE_NUMBER_RIGHT: u32 = 7;
const TITLE_BAR_PADDING: u32 = 10;
const WINDOW_CONTROLS_WIDTH: u32 = 120;
const WINDOW_CONTROLS_HEIGHT: u32 = 40;

const CODE_LINE_PADDING: u32 = 2;
const CODE_PADDING: u32 = 25;

pub struct ImageCode<'a> {
    pub font: FontCollection,
    pub config: CodeConfig,
    pub syntax_set: &'a SyntaxSet,
    pub syntax: &'a SyntaxReference,
    pub theme: Cow<'a, Theme>,
    pub content: &'a str,
}

impl<'a> ImageCode<'a> {
    /// calculate the height of a line
    fn get_line_height(&self) -> u32 {
        self.font.get_font_height() + LINE_SPACE
    }

    /// calculate the Y coordinate of a line
    fn get_line_y(&self, lineno: u32) -> u32 {
        lineno * self.get_line_height()
            + CODE_PADDING
            + if self.config.window_controls || self.config.window_title.is_some() {
                WINDOW_CONTROLS_HEIGHT + TITLE_BAR_PADDING
            } else {
                0
            }
    }

    /// Calculate where code start
    fn get_left_pad(&self, lineno: u32) -> u32 {
        CODE_PADDING
            + if self.config.line_numbers {
                let tmp = format!(
                    "{:>width$}",
                    0,
                    width = lineno.checked_ilog10().unwrap_or_default() as usize + 1
                );
                2 * LINE_NUMBER_RIGHT + self.font.get_text_len(&tmp)
            } else {
                0
            }
    }

    fn create_line(
        &self,
        n: usize,
        hi: bool,
        max_lineno: u32,
        mut fg: Color,
        tab: &str,
        tokens: &[(Style, &str)],
        max_width: &mut u32,
    ) -> Vec<Drawable> {
        let height = self.get_line_y(n as u32);
        let mut width = self.get_left_pad(max_lineno);
        let mut drawables = Vec::new();

        fg.a /= 2;

        for (style, text) in tokens {
            let text = text.trim_end_matches('\n').replace('\t', tab);
            if text.is_empty() {
                continue;
            }
            drawables.push((
                width,
                height,
                Some(if !hi { style.foreground } else { fg }),
                style.font_style.into(),
                text.to_owned(),
            ));

            width += self.font.get_text_len(&text);

            *max_width = (*max_width).max(width);
        }

        drawables
    }

    fn draw_line_number(
        &self,
        img: &mut DynamicImage,
        lines: Range<usize>,
        lineno: u32,
        mut color: Rgba<u8>,
    ) {
        for i in color.0.iter_mut() {
            *i = (*i).saturating_sub(20);
        }
        for (i, l) in lines.enumerate() {
            let line_mumber = format!(
                "{:>width$}",
                l + 1,
                width = lineno.checked_ilog10().unwrap_or_default() as usize + 1
            );
            self.font.draw_text_mut(
                img,
                color,
                CODE_PADDING,
                self.get_line_y(i as u32),
                FontStyle::REGULAR,
                &line_mumber,
            );
        }
    }

    fn highlight_lines<I: IntoIterator<Item = u32>>(&self, img: &mut DynamicImage, lines: I) {
        let width = img.width();
        let height = self.font.get_font_height() + LINE_SPACE;
        let mut color = img.get_pixel(20, 20);

        for i in color.0.iter_mut() {
            *i = (*i).saturating_add(40);
        }

        let shadow = RgbaImage::from_pixel(width, height, color);

        for i in lines {
            let y = self.get_line_y(i - 1);
            copy_alpha(&shadow, img.as_mut_rgba8().unwrap(), 0, y);
        }
    }
}

impl<'a> DynImageContent for ImageCode<'a> {
    fn content(&self) -> DynamicImage {
        let mut h = HighlightLines::new(self.syntax, &self.theme);
        let mut drawables = Vec::new();
        let mut max_width = 0;

        let foreground = self
            .theme
            .settings
            .highlight
            .or(self.theme.settings.foreground)
            .unwrap();
        let background = self.theme.settings.background.unwrap();
        let tab = " ".repeat(self.config.tab_width as usize);
        let lines = self.content.split('\n').collect::<Vec<&str>>();
        let line_range = self
            .config
            .lines
            .clone()
            .map(|l| Range {
                end: l.end.min(lines.len()),
                ..l
            })
            .unwrap_or_default();
        let line_hi = self
            .config
            .highlight_lines
            .clone()
            .map(|l| Range {
                end: l.end.min(lines.len()),
                ..l
            })
            .unwrap_or_default();
        let max_lineno = line_range.end as u32;

        if let Some(title) = self.config.window_title.as_ref() {
            let title_width = self.font.get_text_len(title);
            drawables.push((
                TITLE_BAR_PADDING
                    + if self.config.window_controls {
                        WINDOW_CONTROLS_WIDTH + TITLE_BAR_PADDING
                    } else {
                        self.get_left_pad(max_lineno) + TITLE_BAR_PADDING
                    },
                TITLE_BAR_PADDING + (WINDOW_CONTROLS_HEIGHT / 2) - self.font.get_font_height() / 2,
                None,
                FontStyle::BOLD,
                title.to_string(),
            ));
            max_width = max_width.max(WINDOW_CONTROLS_WIDTH + title_width + TITLE_BAR_PADDING * 2)
        }

        for (n, line) in lines[line_range.clone()].iter().enumerate() {
            let line = h.highlight_line(line, self.syntax_set).unwrap();
            let hi = !line_hi.contains(&n);
            drawables.extend(self.create_line(
                n,
                hi,
                max_lineno,
                foreground,
                &tab,
                &line,
                &mut max_width,
            ));
        }

        let size = (
            max_width + CODE_PADDING,
            self.get_line_y(max_lineno) + CODE_PADDING,
        );

        let mut img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            size.0,
            size.1,
            color_to_rgba(background),
        ));

        // Draw line numbers
        if self.config.line_numbers {
            self.draw_line_number(&mut img, line_range, max_lineno, color_to_rgba(foreground));
        }

        // Draw lines
        for (x, y, color, style, text) in &drawables {
            let color = color_to_rgba(color.unwrap_or(foreground));
            self.font
                .draw_text_mut(&mut img, color, *x, *y, *style, &text);
        }

        // Draw window controlls
        if self.config.window_controls {
            add_window_controls(
                &mut img,
                WINDOW_CONTROLS_WIDTH,
                WINDOW_CONTROLS_HEIGHT,
                TITLE_BAR_PADDING,
                WINDOW_CONTROLS_WIDTH / 3 / 4,
            );
        }

        img
    }
}
