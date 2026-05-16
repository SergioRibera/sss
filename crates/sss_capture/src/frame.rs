//! Captured pixel buffer.

use std::path::Path;

use image::RgbaImage;

use crate::error::{CaptureError, Result};

/// A captured frame.
///
/// `Image` is a thin newtype around [`image::RgbaImage`] so the public surface
/// never leaks a transitive crate type. The full `image` crate is re-exported
/// at the crate root for callers who need it.
#[derive(Clone, Debug)]
pub struct Image {
    inner: RgbaImage,
}

impl Image {
    #[inline]
    pub fn new(buf: RgbaImage) -> Self {
        Self { inner: buf }
    }

    /// Allocate a transparent image of the given size.
    pub fn transparent(width: u32, height: u32) -> Self {
        Self::new(RgbaImage::from_pixel(
            width,
            height,
            image::Rgba([0, 0, 0, 0]),
        ))
    }

    #[inline]
    pub fn width(&self) -> u32 {
        self.inner.width()
    }
    #[inline]
    pub fn height(&self) -> u32 {
        self.inner.height()
    }

    #[inline]
    pub fn as_rgba(&self) -> &RgbaImage {
        &self.inner
    }
    #[inline]
    pub fn as_rgba_mut(&mut self) -> &mut RgbaImage {
        &mut self.inner
    }
    #[inline]
    pub fn into_rgba(self) -> RgbaImage {
        self.inner
    }

    /// Encode and write the image to disk. The format is inferred from the
    /// file extension.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.inner
            .save(path.as_ref())
            .map_err(|e| CaptureError::ImageConversion(e.to_string()))
    }
}

impl From<RgbaImage> for Image {
    fn from(b: RgbaImage) -> Self {
        Self::new(b)
    }
}

impl From<Image> for RgbaImage {
    fn from(i: Image) -> Self {
        i.inner
    }
}
