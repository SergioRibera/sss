//! Multi-monitor composition.
//!
//! Used by every backend that doesn't have a single-shot region capture API:
//! we ask the backend to capture every overlapping monitor individually, then
//! rotate / scale-resample / paste each result into the destination buffer.

use image::imageops::{overlay, resize, rotate180, rotate270, rotate90, FilterType};
use image::{Rgba, RgbaImage};

use crate::backend::Backend;
use crate::error::{CaptureError, Result};
use crate::geometry::{Rect, Rotation};
use crate::monitor::Monitor;
use crate::options::CaptureOptions;

/// Rotate / flip an image so it matches the on-screen orientation of the
/// originating monitor.
pub(crate) fn apply_transform(img: RgbaImage, rotation: Rotation) -> RgbaImage {
    let rotated = match rotation {
        Rotation::Normal | Rotation::Flipped => img,
        Rotation::Rotate90 | Rotation::Flipped90 => rotate90(&img),
        Rotation::Rotate180 | Rotation::Flipped180 => rotate180(&img),
        Rotation::Rotate270 | Rotation::Flipped270 => rotate270(&img),
    };
    if rotation.is_flipped() {
        image::imageops::flip_horizontal(&rotated)
    } else {
        rotated
    }
}

/// Resize an image to a target logical-pixel size using Lanczos3.
fn to_logical(img: RgbaImage, width: u32, height: u32) -> RgbaImage {
    if img.width() == width && img.height() == height {
        return img;
    }
    resize(&img, width, height, FilterType::Lanczos3)
}

/// Capture every monitor and stitch them into a single image whose origin is
/// the top-left of the **bounding box** of all monitors.
pub(crate) fn all_monitors(backend: &dyn Backend, opts: &CaptureOptions) -> Result<RgbaImage> {
    let monitors = backend.monitors()?;
    if monitors.is_empty() {
        return Err(CaptureError::NoMonitors);
    }
    let bounds = Rect::bounding(&monitors.iter().map(|m| m.bounds).collect::<Vec<_>>())
        .ok_or(CaptureError::NoMonitors)?;
    compose(backend, &monitors, bounds, opts)
}

/// Capture the slice of the desktop that falls inside `region`.
pub(crate) fn region(
    backend: &dyn Backend,
    region: Rect,
    opts: &CaptureOptions,
) -> Result<RgbaImage> {
    if region.size.is_empty() {
        return Err(CaptureError::EmptyRegion(region));
    }
    let monitors = backend.monitors()?;
    let touched: Vec<Monitor> = monitors
        .into_iter()
        .filter(|m| m.bounds.intersection(&region).is_some())
        .collect();
    if touched.is_empty() {
        return Err(CaptureError::RegionOutsideDesktop(region));
    }
    compose(backend, &touched, region, opts)
}

fn compose(
    backend: &dyn Backend,
    monitors: &[Monitor],
    output_bounds: Rect,
    opts: &CaptureOptions,
) -> Result<RgbaImage> {
    let mut result = RgbaImage::from_pixel(
        output_bounds.width(),
        output_bounds.height(),
        Rgba([0, 0, 0, 255]),
    );

    for monitor in monitors {
        let m_bounds = monitor.bounds;
        let intersection = match m_bounds.intersection(&output_bounds) {
            Some(r) => r,
            None => continue,
        };

        let raw = match backend.capture_monitor(monitor.id, opts) {
            Ok(img) => img,
            Err(e) => {
                tracing::warn!(monitor = ?monitor.id, error = %e, "monitor capture failed; skipping");
                continue;
            }
        };

        // 1. Bring panel into on-screen orientation.
        let rotated = apply_transform(raw, monitor.rotation);
        // 2. Resample to logical size so coordinates line up.
        let logical = to_logical(rotated, m_bounds.width(), m_bounds.height());

        // 3. Crop to the intersection in monitor-local coordinates.
        let local_x = (intersection.origin.x - m_bounds.origin.x).max(0) as u32;
        let local_y = (intersection.origin.y - m_bounds.origin.y).max(0) as u32;
        let crop_w = intersection
            .width()
            .min(logical.width().saturating_sub(local_x));
        let crop_h = intersection
            .height()
            .min(logical.height().saturating_sub(local_y));
        if crop_w == 0 || crop_h == 0 {
            continue;
        }
        let crop = image::imageops::crop_imm(&logical, local_x, local_y, crop_w, crop_h).to_image();

        // 4. Paste into the output buffer.
        let place_x = (intersection.origin.x - output_bounds.origin.x) as i64;
        let place_y = (intersection.origin.y - output_bounds.origin.y) as i64;
        overlay(&mut result, &crop, place_x, place_y);
    }

    Ok(result)
}

/// Reverse the rotation reported by the platform: given an image in logical
/// (on-screen) orientation, rotate it back into the panel's native
/// orientation. Used by the `capture_region` algorithm to feed
/// logical-coordinate crops back to backends that operate in panel pixels.
#[allow(dead_code)]
pub(crate) fn inverse_transform(img: RgbaImage, rotation: Rotation) -> RgbaImage {
    let undone = match rotation {
        Rotation::Normal | Rotation::Flipped => img,
        Rotation::Rotate90 | Rotation::Flipped90 => rotate270(&img),
        Rotation::Rotate180 | Rotation::Flipped180 => rotate180(&img),
        Rotation::Rotate270 | Rotation::Flipped270 => rotate90(&img),
    };
    if rotation.is_flipped() {
        image::imageops::flip_horizontal(&undone)
    } else {
        undone
    }
}
