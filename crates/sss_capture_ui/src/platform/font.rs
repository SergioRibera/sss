//! Cached font rasteriser for the wayland-layer-shell overlay.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};

const FONT_BYTES: &[u8] = include_bytes!("../../../../assets/fonts/Hack-Regular.ttf");

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
