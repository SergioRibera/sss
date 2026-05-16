//! macOS backend, written directly on top of CoreGraphics through the
//! [`core-graphics`] and [`core-foundation`] bindings. No third-party capture
//! library is involved.
//!
//! Capabilities:
//! * Display enumeration via `CGGetActiveDisplayList`, with bounds /
//!   refresh-rate / rotation pulled out of `CGDisplayBounds`,
//!   `CGDisplayCopyDisplayMode` and `CGDisplayRotation`.
//! * Frame capture via `CGDisplayCreateImage` / `CGDisplayCreateImageForRect`.
//! * Window enumeration via `CGWindowListCopyWindowInfo`; window capture via
//!   `CGWindowListCreateImage`.
//! * Cursor position via `NSEvent.mouseLocation` (Cocoa).

#![allow(non_snake_case)]

use std::sync::Mutex;

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, ItemRef, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_graphics::display::{
    kCGNullWindowID, kCGWindowImageBoundsIgnoreFraming, kCGWindowImageDefault,
    kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, CFArrayRef,
    CGDirectDisplayID, CGDisplay, CGDisplayBounds, CGError, CGGetActiveDisplayList,
    CGMainDisplayID, CGPoint, CGRect, CGSize, CGWindowID, CGWindowListCopyWindowInfo,
    CGWindowListCreateImage,
};
use core_graphics::geometry::CG_ZERO_RECT;
use core_graphics::image::CGImage;
use image::RgbaImage;

use crate::backend::compose;
use crate::backend::Backend;
use crate::error::{CaptureError, Result};
use crate::geometry::{Point, Rect, Rotation};
use crate::monitor::{Monitor, MonitorId};
use crate::options::CaptureOptions;
use crate::window::{Window, WindowId};

const BACKEND: &str = "macos-cg";

pub(crate) struct MacOsBackend {
    _lock: Mutex<()>,
}

impl MacOsBackend {
    pub fn try_new() -> Result<Self> {
        // Probe: at least one active display.
        let mut count: u32 = 0;
        let err = unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count) };
        if err != 0 {
            return Err(CaptureError::backend(
                BACKEND,
                format!("CGGetActiveDisplayList probe failed: {err}"),
            ));
        }
        if count == 0 {
            return Err(CaptureError::NoMonitors);
        }
        Ok(Self {
            _lock: Mutex::new(()),
        })
    }
}

impl Backend for MacOsBackend {
    fn name(&self) -> &'static str {
        BACKEND
    }

    fn monitors(&self) -> Result<Vec<Monitor>> {
        let displays = active_displays()?;
        let main = unsafe { CGMainDisplayID() };
        let mut out = Vec::with_capacity(displays.len());
        for id in displays {
            let bounds = unsafe { CGDisplayBounds(id) };
            let rotation = unsafe { CGDisplay::new(id).rotation() };
            let refresh = display_refresh_rate(id);
            let pixel_size = display_pixel_size(id);
            let scale = if bounds.size.width > 0.0 {
                pixel_size.0 as f32 / bounds.size.width as f32
            } else {
                1.0
            };
            out.push(Monitor {
                id: MonitorId(id as u64),
                name: format!("Display {id}"),
                bounds: Rect::from_xywh(
                    bounds.origin.x as i32,
                    bounds.origin.y as i32,
                    bounds.size.width as u32,
                    bounds.size.height as u32,
                ),
                physical_size: pixel_size,
                scale_factor: scale,
                rotation: Rotation::from_degrees(rotation as f32),
                refresh_rate: refresh,
                is_primary: id == main,
            });
        }
        if out.is_empty() {
            return Err(CaptureError::NoMonitors);
        }
        Ok(out)
    }

    fn windows(&self) -> Result<Vec<Window>> {
        enumerate_windows()
    }

    fn capture_monitor(&self, id: MonitorId, _opts: &CaptureOptions) -> Result<RgbaImage> {
        let display = CGDisplay::new(id.raw() as CGDirectDisplayID);
        let img = display
            .image()
            .ok_or_else(|| CaptureError::backend(BACKEND, "CGDisplayCreateImage returned null"))?;
        cgimage_to_rgba(&img)
    }

    fn capture_window(&self, id: WindowId, _opts: &CaptureOptions) -> Result<RgbaImage> {
        let cgid = id.raw() as CGWindowID;
        let img = unsafe {
            CGWindowListCreateImage(
                CG_ZERO_RECT,
                kCGWindowListOptionOnScreenOnly,
                cgid,
                kCGWindowImageBoundsIgnoreFraming | kCGWindowImageDefault,
            )
        };
        if img.is_null() {
            return Err(CaptureError::WindowNotFound(id));
        }
        // SAFETY: CGWindowListCreateImage returned non-null; wrap into RAII type.
        let cgimg = unsafe { CGImage::from_ptr(img) };
        cgimage_to_rgba(&cgimg)
    }

    fn capture_all(&self, opts: &CaptureOptions) -> Result<RgbaImage> {
        compose::all_monitors(self, opts)
    }

    fn capture_region(&self, region: Rect, opts: &CaptureOptions) -> Result<RgbaImage> {
        if region.size.is_empty() {
            return Err(CaptureError::EmptyRegion(region));
        }
        // Try the single-display fast path first.
        let displays = active_displays()?;
        for id in &displays {
            let bounds_cg = unsafe { CGDisplayBounds(*id) };
            let bounds = Rect::from_xywh(
                bounds_cg.origin.x as i32,
                bounds_cg.origin.y as i32,
                bounds_cg.size.width as u32,
                bounds_cg.size.height as u32,
            );
            if let Some(inter) = bounds.intersection(&region) {
                if inter == region {
                    // CGDisplayCreateImageForRect wants coordinates relative
                    // to the display, but expressed in screen (top-left)
                    // coordinates — Core Graphics uses the same coordinate
                    // system as `CGDisplayBounds` here.
                    let cg_rect = CGRect {
                        origin: CGPoint {
                            x: region.x() as f64,
                            y: region.y() as f64,
                        },
                        size: CGSize {
                            width: region.width() as f64,
                            height: region.height() as f64,
                        },
                    };
                    if let Some(img) = CGDisplay::new(*id).image_for_rect(cg_rect) {
                        return cgimage_to_rgba(&img);
                    }
                }
            }
        }
        compose::region(self, region, opts)
    }

    fn cursor_position(&self) -> Result<Point> {
        // Use objc2-app-kit to call NSEvent.mouseLocation. We avoid pulling in
        // a full objc runtime wrapper just for one call by going through the
        // raw `msg_send!` macro.
        use objc2::msg_send_id;
        use objc2::runtime::AnyClass;
        use objc2_foundation::NSPoint;

        let cls = AnyClass::get("NSEvent")
            .ok_or_else(|| CaptureError::CursorUnavailable("NSEvent class missing".into()))?;
        // SAFETY: NSEvent.mouseLocation is a class method returning NSPoint.
        let point: NSPoint = unsafe { objc2::msg_send![cls, mouseLocation] };
        // Convert from Cocoa (origin at bottom-left of main display) to
        // top-left coordinates used everywhere else in this crate.
        let main_bounds = unsafe { CGDisplayBounds(CGMainDisplayID()) };
        let _ = msg_send_id::<()>; // silence unused-import lint on some paths
        Ok(Point::new(
            point.x as i32,
            (main_bounds.size.height - point.y) as i32,
        ))
    }
}

// -----------------------------------------------------------------------------
// CGImage → RgbaImage
// -----------------------------------------------------------------------------

fn cgimage_to_rgba(img: &CGImage) -> Result<RgbaImage> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let bytes_per_row = img.bytes_per_row() as u32;
    let data = img.data();
    let bytes = data.bytes();

    // CGImage may have padding at the end of each row.
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height as usize {
        let start = y * bytes_per_row as usize;
        let row = &bytes[start..start + (width * 4) as usize];
        // CG uses BGRA on little-endian.
        for px in row.chunks_exact(4) {
            out.push(px[2]); // R
            out.push(px[1]); // G
            out.push(px[0]); // B
            out.push(px[3]); // A
        }
    }

    RgbaImage::from_raw(width, height, out).ok_or_else(|| {
        CaptureError::ImageConversion(format!("CGImage buffer mismatch {width}x{height}"))
    })
}

// -----------------------------------------------------------------------------
// Display enumeration
// -----------------------------------------------------------------------------

fn active_displays() -> Result<Vec<CGDirectDisplayID>> {
    let mut count: u32 = 0;
    let err = unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count) };
    if err != 0 {
        return Err(CaptureError::backend(
            BACKEND,
            format!("CGGetActiveDisplayList: {err}"),
        ));
    }
    if count == 0 {
        return Err(CaptureError::NoMonitors);
    }
    let mut ids = vec![0u32; count as usize];
    let err = unsafe { CGGetActiveDisplayList(count, ids.as_mut_ptr(), &mut count) };
    if err != 0 {
        return Err(CaptureError::backend(
            BACKEND,
            format!("CGGetActiveDisplayList: {err}"),
        ));
    }
    ids.truncate(count as usize);
    Ok(ids)
}

fn display_pixel_size(id: CGDirectDisplayID) -> (u32, u32) {
    let display = CGDisplay::new(id);
    let mode = match display.display_mode() {
        Some(m) => m,
        None => return (display.pixels_wide() as u32, display.pixels_high() as u32),
    };
    (mode.pixel_width() as u32, mode.pixel_height() as u32)
}

fn display_refresh_rate(id: CGDirectDisplayID) -> Option<f32> {
    let display = CGDisplay::new(id);
    let mode = display.display_mode()?;
    let hz = mode.refresh_rate();
    if hz > 0.0 {
        Some(hz as f32)
    } else {
        // Built-in displays report 0; fall back to the system refresh value.
        None
    }
}

// -----------------------------------------------------------------------------
// Window enumeration
// -----------------------------------------------------------------------------

fn enumerate_windows() -> Result<Vec<Window>> {
    let opts = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let list_ptr: CFArrayRef = unsafe { CGWindowListCopyWindowInfo(opts, kCGNullWindowID) };
    if list_ptr.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: list_ptr is non-null and owned (Create rule).
    let array: CFArray<CFDictionary<CFString, CFType>> =
        unsafe { CFArray::wrap_under_create_rule(list_ptr) };
    let mut out = Vec::with_capacity(array.len() as usize);
    for i in 0..array.len() {
        let item: ItemRef<'_, CFDictionary<CFString, CFType>> = match array.get(i) {
            Some(v) => v,
            None => continue,
        };
        let title = string_value(&item, "kCGWindowName").unwrap_or_default();
        let app = string_value(&item, "kCGWindowOwnerName").unwrap_or_default();
        let window_id = number_value::<u64>(&item, "kCGWindowNumber").unwrap_or(0);
        let bounds = bounds_value(&item).unwrap_or_default();
        out.push(Window {
            id: WindowId(window_id),
            title,
            app_name: app,
            bounds,
            monitor: None,
            is_minimized: false,
            is_maximized: false,
            is_focused: false,
        });
    }
    Ok(out)
}

fn string_value(dict: &CFDictionary<CFString, CFType>, key: &str) -> Option<String> {
    let cf_key = CFString::new(key);
    let val = dict.find(&cf_key)?;
    let s: CFString = val.downcast::<CFString>()?;
    Some(s.to_string())
}

fn number_value<T: From<i64>>(dict: &CFDictionary<CFString, CFType>, key: &str) -> Option<T> {
    let cf_key = CFString::new(key);
    let val = dict.find(&cf_key)?;
    let n: CFNumber = val.downcast::<CFNumber>()?;
    n.to_i64().map(T::from)
}

fn bounds_value(dict: &CFDictionary<CFString, CFType>) -> Option<Rect> {
    let cf_key = CFString::new("kCGWindowBounds");
    let val = dict.find(&cf_key)?;
    let bdict: CFDictionary<CFString, CFType> = val.downcast::<CFDictionary<CFString, CFType>>()?;
    let x = number_value::<i64>(&bdict, "X")? as i32;
    let y = number_value::<i64>(&bdict, "Y")? as i32;
    let w = number_value::<i64>(&bdict, "Width")? as u32;
    let h = number_value::<i64>(&bdict, "Height")? as u32;
    Some(Rect::from_xywh(x, y, w, h))
}
