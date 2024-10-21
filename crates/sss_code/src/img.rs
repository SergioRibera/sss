//! This file is inspired from https://github.com/Aloxaf/silicon
//!
//! Source from: https://github.com/Aloxaf/silicon/blob/master/src/formatter.rs
//!
use std::borrow::Cow;
use std::ops::Range;

use sss_lib::error::ImagenGeneration as ImagenGenerationError;
use sss_lib::font::{FontCollection, FontStyle};
use sss_lib::image::{Rgba, RgbaImage};
use sss_lib::{Background, DynImageContent, GenerationSettings};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Color, Style, Theme};
use syntect::parsing::{SyntaxReference, SyntaxSet};

use crate::config::CodeConfig;
use crate::error::Configuration as ConfigurationError;
use crate::utils::{color_to_rgba, fontstyle_from_syntect};

type Drawable = (u32, u32, Option<Color>, FontStyle, String);

const LINE_SPACE: u32 = 0;
const LINE_NUMBER_RIGHT: u32 = 7;

const CODE_PADDING: u32 = 25;

pub struct ImageCode<'a> {
    pub font: FontCollection,
    pub lib_config: GenerationSettings,
    pub config: CodeConfig,
    pub syntax_set: &'a SyntaxSet,
    pub syntax: &'a SyntaxReference,
    pub theme: Cow<'a, Theme>,
    pub content: &'a str,
}

impl<'a> ImageCode<'a> {
    /// calculate the height of a line
    fn get_line_height(&self) -> Result<u32, ImagenGenerationError> {
        Ok(self.font.get_font_height()? + LINE_SPACE)
    }

    /// calculate the Y coordinate of a line
    fn get_line_y(&self, lineno: u32, max_h: u32) -> Result<u32, ImagenGenerationError> {
        Ok((lineno * self.get_line_height()?) + max_h)
    }

    /// Calculate where code start
    fn get_left_pad(&self, lineno: u32) -> Result<u32, ImagenGenerationError> {
        Ok(CODE_PADDING
            + if self.config.line_numbers {
                let tmp = format!(
                    "{:>width$}",
                    0,
                    width = lineno.checked_ilog10().unwrap_or_default() as usize + 1
                );
                2 * LINE_NUMBER_RIGHT + self.font.get_text_len(&tmp)?
            } else {
                0
            })
    }

    fn create_line(
        &self,
        (n, hi, max_lineno): (usize, bool, u32),
        mut fg: Color,
        tab: &str,
        tab_size: usize,
        indents: &[char],
        tokens: &[(Style, &str)],
        max_width: &mut u32,
        max_h: u32,
    ) -> Result<Vec<Drawable>, ImagenGenerationError> {
        let height = self.get_line_y(n as u32, max_h)?;
        let mut width = self.get_left_pad(max_lineno)?;
        let mut drawables = Vec::new();
        let get_tab = |level: usize| {
            if indents.is_empty() || level == 0 {
                tab.to_owned()
            } else {
                (0..level)
                    .map(|level| {
                        let indent_char = indents[level % indents.len()];
                        format!("{}{:width$}", indent_char, " ", width = tab_size - 1)
                    })
                    .collect::<String>()
            }
        };

        fg.a /= 2;

        let indent_level = tokens
            .get(0)
            .map(|(_, text)| text.split('\t').count() + text.split(tab).count())
            .unwrap_or_default()
            .saturating_sub(2);
        let indent = get_tab(indent_level);
        tracing::debug!("Indent Level for line {n}: {indent_level} - {indent}");
        drawables.push((width, height, Some(fg), FontStyle::Regular, indent));

        for (style, text) in tokens {
            let text = text.trim_end_matches('\n');
            if text.is_empty() {
                continue;
            }
            drawables.push((
                width,
                height,
                Some(if hi { style.foreground } else { fg }),
                fontstyle_from_syntect(style.font_style),
                text.to_owned(),
            ));

            width += self.font.get_text_len(&text)?;

            *max_width = (*max_width).max(width);
        }

        Ok(drawables)
    }

    fn draw_line_number(
        &self,
        img: &mut RgbaImage,
        lines: Range<usize>,
        line_hi: Range<usize>,
        lineno: u32,
        max_h: u32,
        mut color: Rgba<u8>,
    ) -> Result<(), ImagenGenerationError> {
        for i in color.0.iter_mut() {
            *i = (*i).saturating_sub(20);
        }
        let no_hi_color = {
            let mut c = color;
            c.0[3] /= 2;
            c
        };
        for (i, l) in lines.clone().enumerate() {
            let line_mumber = format!(
                "{:>width$}",
                l + 1,
                width = lineno.checked_ilog10().unwrap_or_default() as usize + 1
            );
            self.font.draw_text_mut(
                img,
                if line_hi.contains(&(lines.start + i)) {
                    color
                } else {
                    no_hi_color
                },
                CODE_PADDING,
                self.get_line_y(i as u32, max_h)?,
                FontStyle::Regular,
                &line_mumber,
            )?;
        }

        Ok(())
    }
}

impl<'a> DynImageContent for ImageCode<'a> {
    fn content(&self) -> Result<RgbaImage, ImagenGenerationError> {
        tracing::trace!("Generating Image");
        let mut h = HighlightLines::new(self.syntax, &self.theme);
        let mut drawables = Vec::new();
        let mut max_width = 0;

        let foreground = self
            .theme
            .settings
            .highlight
            .or(self.theme.settings.foreground)
            .ok_or(ImagenGenerationError::Custom(
                ConfigurationError::ParamNotFound("theme.foreground".to_owned()).to_string(),
            ))?;
        tracing::debug!("Default Foreground of Theme {foreground:?}");
        let background = self
            .config
            .code_background
            .clone()
            .and_then(|b| Background::try_from(b).ok())
            .or(self
                .theme
                .settings
                .background
                .map(|b| Background::Solid(Rgba([b.r, b.g, b.b, b.a]))))
            .ok_or(ImagenGenerationError::Custom(
                ConfigurationError::ParamNotFound("theme.code_background".to_owned()).to_string(),
            ))?;
        tracing::debug!("Default Background {background:?}");
        tracing::debug!("Indent Chars: {:?}", &self.config.indent_chars);
        let tab_width = (self.config.tab_width.unwrap_or(4)) as usize;
        let tab = " ".repeat(tab_width);
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
        tracing::debug!("Line Range: {line_range:?}");
        let line_hi = self
            .config
            .highlight_lines
            .clone()
            .map(|l| Range {
                end: l.end.min(lines.len()),
                ..l
            })
            .unwrap_or_default();
        tracing::debug!("Lines to highlight: {line_hi:?}");
        let max_lineno = line_range.len() as u32;
        tracing::debug!("Max Line number: {max_lineno:?}");
        let max_h_controls = if self.lib_config.window_controls.enable
            || self.lib_config.window_controls.title.is_some()
        {
            LINE_SPACE
        } else {
            CODE_PADDING
        };

        for (n, line) in lines[line_range.clone()].iter().enumerate() {
            let line = h
                .highlight_line(line, self.syntax_set)
                .map_err(|e| ImagenGenerationError::Custom(e.to_string()))?;
            let hi = line_hi.contains(&(line_range.start + n));
            drawables.extend(self.create_line(
                (n, hi, max_lineno),
                foreground,
                &tab,
                tab_width,
                &self.config.indent_chars,
                &line,
                &mut max_width,
                max_h_controls,
            )?);
        }

        let size = (
            max_width + CODE_PADDING,
            self.get_line_y(max_lineno, max_h_controls)? + CODE_PADDING,
        );
        tracing::debug!("Size code image: {size:?}");

        let mut img = background.to_image(size.0, size.1);

        // Draw line numbers
        if self.config.line_numbers {
            tracing::trace!("Draw line numbers");
            self.draw_line_number(
                &mut img,
                line_range,
                line_hi,
                max_lineno,
                max_h_controls,
                color_to_rgba(foreground),
            )?;
        }

        // Draw lines
        tracing::trace!("Draw code");
        for (x, y, color, style, text) in &drawables {
            let color = color_to_rgba(color.unwrap_or(foreground));
            self.font
                .draw_text_mut(&mut img, color, *x, *y, *style, text)?;
        }

        Ok(img)
    }
}
