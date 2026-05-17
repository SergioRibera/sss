//! Cached TTF rasteriser shared by the wayland-layer-shell overlay and the
//! CPU compositor that bakes annotations into the captured image.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};

const FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/Hack-Regular.ttf");

fn font() -> &'static FontRef<'static> {
    static FONT: OnceLock<FontRef<'static>> = OnceLock::new();
    FONT.get_or_init(|| FontRef::try_from_slice(FONT_BYTES).expect("Hack-Regular.ttf is malformed"))
}

/// Row-major coverage bitmap (0 = transparent, 255 = fully covered).
pub(crate) struct GlyphBitmap {
    pub width: u32,
    pub height: u32,
    pub bearing_x: i32,
    pub bearing_y: i32,
    pub advance: f32,
    pub pixels: Vec<u8>,
}

pub(crate) fn glyph_for(ch: char, px: f32) -> Option<&'static GlyphBitmap> {
    let key = (ch, (px * 16.0) as u32);
    let guard = cache().lock().unwrap();
    // SAFETY: cache entries are never removed; addresses are stable forever.
    if let Some(g) = guard.get(&key) {
        return g.as_ref().map(|g| unsafe { &*(g as *const _) });
    }
    drop(guard);
    let raster = rasterise(ch, px);
    let mut guard = cache().lock().unwrap();
    let inserted = guard.entry(key).or_insert(raster);
    inserted.as_ref().map(|g| unsafe { &*(g as *const _) })
}

type CharGlyphCache = HashMap<(char, u32), Option<GlyphBitmap>>;

fn cache() -> &'static Mutex<CharGlyphCache> {
    static CACHE: OnceLock<Mutex<CharGlyphCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn rasterise(ch: char, px: f32) -> Option<GlyphBitmap> {
    let font = font();
    let scaled = font.as_scaled(PxScale::from(px));
    let glyph_id = font.glyph_id(ch);
    let glyph = glyph_id.with_scale(PxScale::from(px));
    let outlined = font.outline_glyph(glyph)?;
    let bounds = outlined.px_bounds();
    let width = bounds.width().ceil() as u32;
    let height = bounds.height().ceil() as u32;
    let mut pixels = vec![0u8; (width * height) as usize];
    outlined.draw(|x, y, c| {
        let idx = (y * width + x) as usize;
        if idx < pixels.len() {
            pixels[idx] = (c * 255.0) as u8;
        }
    });
    Some(GlyphBitmap {
        width,
        height,
        bearing_x: bounds.min.x as i32,
        bearing_y: bounds.min.y as i32,
        advance: scaled.h_advance(glyph_id),
        pixels,
    })
}

pub(crate) fn measure(text: &str, px: f32) -> f32 {
    let font = font();
    let scaled = font.as_scaled(PxScale::from(px));
    text.chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum()
}

pub(crate) fn ascent(px: f32) -> f32 {
    let font = font();
    let scaled = font.as_scaled(PxScale::from(px));
    scaled.ascent()
}

/// Alpha-blend `text` onto an `RgbaImage` at top-left `(x, y)`, modulating the
/// glyph coverage by `color`'s alpha. Used by the CPU compositor that flattens
/// the editor canvas into the captured image.
pub(crate) fn draw_text_rgba(
    img: &mut RgbaImage,
    x: i32,
    y: i32,
    text: &str,
    color: [u8; 4],
    px: f32,
) {
    let ascent = ascent(px);
    let baseline = y as f32 + ascent;
    let (iw, ih) = img.dimensions();
    let mut pen_x = x as f32;
    for ch in text.chars() {
        let glyph = match glyph_for(ch, px) {
            Some(g) => g,
            None => {
                pen_x += measure(&ch.to_string(), px);
                continue;
            }
        };
        let gx0 = pen_x + glyph.bearing_x as f32;
        let gy0 = baseline + glyph.bearing_y as f32;
        for gy in 0..glyph.height {
            for gx in 0..glyph.width {
                let coverage = glyph.pixels[(gy * glyph.width + gx) as usize];
                if coverage == 0 {
                    continue;
                }
                let dx = (gx0 + gx as f32).round() as i32;
                let dy = (gy0 + gy as f32).round() as i32;
                if dx < 0 || dy < 0 || (dx as u32) >= iw || (dy as u32) >= ih {
                    continue;
                }
                let mut src = color;
                src[3] = ((src[3] as u16 * coverage as u16) / 255) as u8;
                let dst = img.get_pixel(dx as u32, dy as u32).0;
                img.put_pixel(dx as u32, dy as u32, Rgba(blend_over(dst, src)));
            }
        }
        pen_x += glyph.advance;
    }
}

#[inline]
fn blend_over(dst: [u8; 4], src: [u8; 4]) -> [u8; 4] {
    let sa = src[3] as u32;
    let inv = 255 - sa;
    [
        ((src[0] as u32 * sa + dst[0] as u32 * inv) / 255) as u8,
        ((src[1] as u32 * sa + dst[1] as u32 * inv) / 255) as u8,
        ((src[2] as u32 * sa + dst[2] as u32 * inv) / 255) as u8,
        (sa + (dst[3] as u32 * inv) / 255).min(255) as u8,
    ]
}
