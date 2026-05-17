//! Windows GDI capture backend.

#![allow(non_snake_case, unsafe_op_in_unsafe_fn)]

use std::cell::RefCell;
use std::mem::size_of;
use std::sync::Mutex;

use image::RgbaImage;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
    EnumDisplayMonitors, GetDC, GetDIBits, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS,
    HBITMAP, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTOPRIMARY, SRCCOPY,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetCursorPos, GetForegroundWindow, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, IsIconic, IsWindowVisible, IsZoomed,
};

use crate::backend::compose;
use crate::backend::Backend;
use crate::error::{CaptureError, Result};
use crate::geometry::{Point, Rect, Rotation};
use crate::monitor::{Monitor, MonitorId};
use crate::options::CaptureOptions;
use crate::window::{Window, WindowId};

const BACKEND: &str = "windows-gdi";

pub(crate) struct WindowsBackend {
    // Guards thread-local buffers used by EnumWindows/EnumDisplayMonitors.
    _lock: Mutex<()>,
}

impl WindowsBackend {
    pub fn try_new() -> Result<Self> {
        unsafe {
            let dc = GetDC(HWND(0));
            if dc.0 == 0 {
                return Err(CaptureError::unsupported(
                    BACKEND,
                    "GetDC(NULL) failed; no desktop available",
                ));
            }
            ReleaseDC(HWND(0), dc);
        }
        Ok(Self {
            _lock: Mutex::new(()),
        })
    }
}

impl Backend for WindowsBackend {
    fn name(&self) -> &'static str {
        BACKEND
    }

    fn monitors(&self) -> Result<Vec<Monitor>> {
        let mut out = enumerate_monitors()?;
        unsafe {
            let primary = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            if let Some(m) = out.iter_mut().find(|m| m.id.raw() == primary.0 as u64) {
                m.is_primary = true;
            }
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
        let monitor = self
            .monitors()?
            .into_iter()
            .find(|m| m.id == id)
            .ok_or(CaptureError::MonitorNotFound(id))?;
        capture_rect(monitor.bounds)
    }

    fn capture_window(&self, id: WindowId, _opts: &CaptureOptions) -> Result<RgbaImage> {
        let hwnd = HWND(id.raw() as isize);
        let mut rect = RECT::default();
        unsafe {
            GetWindowRect(hwnd, &mut rect)
                .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?;
        }
        let bounds = Rect::from_xywh(
            rect.left,
            rect.top,
            (rect.right - rect.left).max(0) as u32,
            (rect.bottom - rect.top).max(0) as u32,
        );
        capture_rect(bounds)
    }

    fn capture_all(&self, _opts: &CaptureOptions) -> Result<RgbaImage> {
        let monitors = self.monitors()?;
        let bounds = Rect::bounding(&monitors.iter().map(|m| m.bounds).collect::<Vec<_>>())
            .ok_or(CaptureError::NoMonitors)?;
        capture_rect(bounds)
    }

    fn capture_region(&self, region: Rect, _opts: &CaptureOptions) -> Result<RgbaImage> {
        if region.size.is_empty() {
            return Err(CaptureError::EmptyRegion(region));
        }
        capture_rect(region).or_else(|_| compose::region(self, region, opts))
    }

    fn cursor_position(&self) -> Result<Point> {
        let mut p = POINT::default();
        let ok = unsafe { GetCursorPos(&mut p as *mut POINT) };
        ok.map_err(|e| CaptureError::CursorUnavailable(e.to_string()))?;
        Ok(Point::new(p.x, p.y))
    }
}

thread_local! {
    static MONITOR_BUF: RefCell<Vec<Monitor>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "system" fn monitor_enum_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    _lparam: LPARAM,
) -> BOOL {
    let mut info: MONITORINFOEXW = std::mem::zeroed();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if !GetMonitorInfoW(hmonitor, &mut info.monitorInfo as *mut MONITORINFO).as_bool() {
        return TRUE;
    }
    let MONITORINFO {
        rcMonitor, dwFlags, ..
    } = info.monitorInfo;
    let width = (rcMonitor.right - rcMonitor.left).max(0) as u32;
    let height = (rcMonitor.bottom - rcMonitor.top).max(0) as u32;
    let device_name = String::from_utf16_lossy(
        &info
            .szDevice
            .iter()
            .take_while(|c| **c != 0)
            .copied()
            .collect::<Vec<_>>(),
    );

    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    let _ = GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
    let scale = dpi_x as f32 / 96.0;

    MONITOR_BUF.with(|cell| {
        cell.borrow_mut().push(Monitor {
            id: MonitorId(hmonitor.0 as u64),
            name: device_name,
            bounds: Rect::from_xywh(rcMonitor.left, rcMonitor.top, width, height),
            physical_size: (width, height),
            scale_factor: scale,
            rotation: Rotation::Normal,
            refresh_rate: None,
            is_primary: (dwFlags & 1) != 0,
        });
    });
    TRUE
}

fn enumerate_monitors() -> Result<Vec<Monitor>> {
    MONITOR_BUF.with(|cell| cell.borrow_mut().clear());
    let ok =
        unsafe { EnumDisplayMonitors(HDC(0), None, Some(monitor_enum_proc), LPARAM(0)).as_bool() };
    if !ok {
        return Err(CaptureError::backend(BACKEND, "EnumDisplayMonitors failed"));
    }
    let out = MONITOR_BUF.with(|cell| cell.borrow_mut().drain(..).collect::<Vec<_>>());
    Ok(out)
}

thread_local! {
    static WINDOW_BUF: RefCell<Vec<Window>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "system" fn window_enum_proc(hwnd: HWND, _lparam: LPARAM) -> BOOL {
    if !IsWindowVisible(hwnd).as_bool() {
        return TRUE;
    }
    let len = GetWindowTextLengthW(hwnd);
    if len == 0 {
        return TRUE;
    }
    let mut buf = vec![0u16; len as usize + 1];
    let copied = GetWindowTextW(hwnd, &mut buf);
    buf.truncate(copied as usize);
    let title = String::from_utf16_lossy(&buf);

    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_err() {
        return TRUE;
    }

    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    let active = GetForegroundWindow();

    WINDOW_BUF.with(|cell| {
        cell.borrow_mut().push(Window {
            id: WindowId(hwnd.0 as u64),
            title,
            app_name: String::new(),
            bounds: Rect::from_xywh(
                rect.left,
                rect.top,
                (rect.right - rect.left).max(0) as u32,
                (rect.bottom - rect.top).max(0) as u32,
            ),
            monitor: Some(MonitorId(monitor.0 as u64)),
            is_minimized: IsIconic(hwnd).as_bool(),
            is_maximized: IsZoomed(hwnd).as_bool(),
            is_focused: hwnd == active,
        });
    });
    TRUE
}

fn enumerate_windows() -> Result<Vec<Window>> {
    WINDOW_BUF.with(|cell| cell.borrow_mut().clear());
    unsafe {
        EnumWindows(Some(window_enum_proc), LPARAM(0))
            .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?;
    }
    Ok(WINDOW_BUF.with(|cell| cell.borrow_mut().drain(..).collect::<Vec<_>>()))
}

fn capture_rect(bounds: Rect) -> Result<RgbaImage> {
    if bounds.size.is_empty() {
        return Err(CaptureError::EmptyRegion(bounds));
    }
    unsafe {
        let screen_dc = GetDC(HWND(0));
        if screen_dc.0 == 0 {
            return Err(CaptureError::backend(BACKEND, "GetDC(NULL) failed"));
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.0 == 0 {
            ReleaseDC(HWND(0), screen_dc);
            return Err(CaptureError::backend(BACKEND, "CreateCompatibleDC failed"));
        }
        let bitmap: HBITMAP =
            CreateCompatibleBitmap(screen_dc, bounds.width() as i32, bounds.height() as i32);
        if bitmap.0 == 0 {
            DeleteDC(mem_dc);
            ReleaseDC(HWND(0), screen_dc);
            return Err(CaptureError::backend(
                BACKEND,
                "CreateCompatibleBitmap failed",
            ));
        }
        let old = SelectObject(mem_dc, bitmap);

        // CAPTUREBLT is required to include layered (transparent) top-level UI.
        let ok = BitBlt(
            mem_dc,
            0,
            0,
            bounds.width() as i32,
            bounds.height() as i32,
            screen_dc,
            bounds.x(),
            bounds.y(),
            SRCCOPY | CAPTUREBLT,
        )
        .is_ok();
        if !ok {
            SelectObject(mem_dc, old);
            DeleteObject(bitmap);
            DeleteDC(mem_dc);
            ReleaseDC(HWND(0), screen_dc);
            return Err(CaptureError::backend(BACKEND, "BitBlt failed"));
        }

        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: bounds.width() as i32,
            biHeight: -(bounds.height() as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0 as u32,
            ..Default::default()
        };

        let stride = (bounds.width() * 4) as usize;
        let mut raw = vec![0u8; stride * bounds.height() as usize];

        let lines = GetDIBits(
            mem_dc,
            bitmap,
            0,
            bounds.height(),
            Some(raw.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        SelectObject(mem_dc, old);
        DeleteObject(bitmap);
        DeleteDC(mem_dc);
        ReleaseDC(HWND(0), screen_dc);

        if lines == 0 {
            return Err(CaptureError::backend(BACKEND, "GetDIBits failed"));
        }

        for chunk in raw.chunks_exact_mut(4) {
            chunk.swap(0, 2);
            // GDI leaves alpha undefined on opaque blits.
            chunk[3] = 255;
        }

        RgbaImage::from_raw(bounds.width(), bounds.height(), raw)
            .ok_or_else(|| CaptureError::ImageConversion(format!("buffer too small for {bounds}")))
    }
}

#[allow(dead_code)]
fn _silence_pcwstr(_: PCWSTR) {}
