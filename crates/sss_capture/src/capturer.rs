//! The public capture entry point.

use crate::backend::Backend;
use crate::error::{CaptureError, Result};
use crate::frame::Image;
use crate::geometry::{Point, Rect};
use crate::monitor::{Monitor, MonitorId};
use crate::options::{BackendKind, CaptureOptions};
use crate::window::{Window, WindowId, WindowSearch};

/// Cross-platform screen capture.
pub struct Capturer {
    backend: Box<dyn Backend>,
    default_options: CaptureOptions,
}

unsafe impl Send for Capturer {}
unsafe impl Sync for Capturer {}

impl std::fmt::Debug for Capturer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Capturer")
            .field("backend", &self.backend.name())
            .field("default_options", &self.default_options)
            .finish()
    }
}

impl Default for Capturer {
    fn default() -> Self {
        Self::builder().build().expect("Cannot create Capturer")
    }
}

impl Capturer {
    pub fn builder() -> CapturerBuilder {
        CapturerBuilder::default()
    }

    pub fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    pub fn default_options(&self) -> &CaptureOptions {
        &self.default_options
    }

    pub fn set_default_options(&mut self, opts: CaptureOptions) {
        self.default_options = opts;
    }

    pub fn monitors(&self) -> Result<Vec<Monitor>> {
        self.backend.monitors()
    }

    pub fn primary_monitor(&self) -> Result<Monitor> {
        let mut mons = self.monitors()?;
        if mons.is_empty() {
            return Err(CaptureError::NoMonitors);
        }
        if let Some(idx) = mons.iter().position(|m| m.is_primary()) {
            return Ok(mons.swap_remove(idx));
        }
        Ok(mons.swap_remove(0))
    }

    pub fn monitor_by_id(&self, id: MonitorId) -> Result<Monitor> {
        self.monitors()?
            .into_iter()
            .find(|m| m.id() == id)
            .ok_or(CaptureError::MonitorNotFound(id))
    }

    pub fn monitor_by_name(&self, name: &str) -> Result<Monitor> {
        let needle = name.to_lowercase();
        self.monitors()?
            .into_iter()
            .find(|m| m.name().to_lowercase().contains(&needle))
            .ok_or_else(|| {
                CaptureError::backend("capturer", format!("no monitor matches {name:?}"))
            })
    }

    pub fn monitor_at(&self, point: Point) -> Result<Monitor> {
        self.monitors()?
            .into_iter()
            .find(|m| m.bounds().contains(point))
            .ok_or(CaptureError::PointOutsideDesktop {
                x: point.x,
                y: point.y,
            })
    }

    pub fn monitor_at_cursor(&self) -> Result<Monitor> {
        let p = self.cursor_position()?;
        self.monitor_at(p)
    }

    pub fn windows(&self) -> Result<Vec<Window>> {
        self.backend.windows()
    }

    pub fn window_by_id(&self, id: WindowId) -> Result<Window> {
        self.windows()?
            .into_iter()
            .find(|w| w.id() == id)
            .ok_or(CaptureError::WindowNotFound(id))
    }

    pub fn window_by_title(&self, needle: &str) -> Result<Window> {
        self.find_window(WindowSearch::by_title(needle))
    }

    pub fn find_window(&self, search: impl Into<WindowSearch>) -> Result<Window> {
        let search = search.into();
        self.windows()?
            .into_iter()
            .find(|w| search.matches(w))
            .ok_or_else(|| CaptureError::backend("capturer", "no window matched the search"))
    }

    pub fn capture_all(&self) -> Result<Image> {
        self.capture_all_with(self.default_options)
    }

    pub fn capture_all_with(&self, opts: CaptureOptions) -> Result<Image> {
        self.backend.capture_all(&opts).map(Image::from)
    }

    pub fn capture_monitor(&self, monitor: &Monitor) -> Result<Image> {
        self.capture_monitor_with(monitor, self.default_options)
    }

    pub fn capture_monitor_with(&self, monitor: &Monitor, opts: CaptureOptions) -> Result<Image> {
        self.backend
            .capture_monitor(monitor.id(), &opts)
            .map(Image::from)
    }

    pub fn capture_window(&self, window: &Window) -> Result<Image> {
        self.capture_window_with(window, self.default_options)
    }

    pub fn capture_window_with(&self, window: &Window, opts: CaptureOptions) -> Result<Image> {
        self.backend
            .capture_window(window.id(), &opts)
            .map(Image::from)
    }

    pub fn capture_region(&self, region: Rect) -> Result<Image> {
        self.capture_region_with(region, self.default_options)
    }

    pub fn capture_region_with(&self, region: Rect, opts: CaptureOptions) -> Result<Image> {
        self.backend.capture_region(region, &opts).map(Image::from)
    }

    pub fn capture_at(&self, point: Point) -> Result<Image> {
        let monitor = self.monitor_at(point)?;
        self.capture_monitor(&monitor)
    }

    pub fn capture_at_cursor(&self) -> Result<Image> {
        let p = self.cursor_position()?;
        self.capture_at(p)
    }

    pub fn cursor_position(&self) -> Result<Point> {
        self.backend.cursor_position()
    }
}

#[derive(Clone, Debug, Default)]
pub struct CapturerBuilder {
    backend: BackendKind,
    options: CaptureOptions,
}

impl CapturerBuilder {
    pub fn backend(mut self, kind: BackendKind) -> Self {
        self.backend = kind;
        self
    }

    pub fn show_cursor(mut self, show: bool) -> Self {
        self.options.show_cursor = show;
        self
    }

    pub fn options(mut self, opts: CaptureOptions) -> Self {
        self.options = opts;
        self
    }

    pub fn build(self) -> Result<Capturer> {
        let backend = select_backend(self.backend)?;
        tracing::info!(backend = backend.name(), "sss_capture: backend selected");
        Ok(Capturer {
            backend,
            default_options: self.options,
        })
    }
}

fn select_backend(kind: BackendKind) -> Result<Box<dyn Backend>> {
    let mut errors: Vec<String> = Vec::new();
    match kind {
        BackendKind::Auto => auto_select(&mut errors),
        BackendKind::Wayland => try_wayland(&mut errors),
        BackendKind::WaylandPortal => try_portal(&mut errors),
        BackendKind::X11 => try_x11(&mut errors),
        BackendKind::WindowsGdi | BackendKind::WindowsDxgi => try_windows(&mut errors),
        BackendKind::MacOS => try_macos(&mut errors),
    }
    .ok_or(CaptureError::NoBackend(errors))
}

#[cfg(target_os = "linux")]
fn auto_select(errors: &mut Vec<String>) -> Option<Box<dyn Backend>> {
    use crate::backend::linux::{is_wayland_session, is_x11_session};

    if is_wayland_session() {
        if let Some(b) = try_wayland(errors) {
            return Some(b);
        }
        // On wlroots compositors without zwlr_screencopy_v1, XWayland is more
        // likely to work than the portal which hangs without a configured
        // `org.freedesktop.portal.Screenshot` backend.
        if is_x11_session() {
            if let Some(b) = try_x11(errors) {
                return Some(b);
            }
        }
        if let Some(b) = try_portal(errors) {
            return Some(b);
        }
        return None;
    }
    if is_x11_session() {
        if let Some(b) = try_x11(errors) {
            return Some(b);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn auto_select(errors: &mut Vec<String>) -> Option<Box<dyn Backend>> {
    try_windows(errors)
}

#[cfg(target_os = "macos")]
fn auto_select(errors: &mut Vec<String>) -> Option<Box<dyn Backend>> {
    try_macos(errors)
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn auto_select(errors: &mut Vec<String>) -> Option<Box<dyn Backend>> {
    errors.push("no capture backend implemented for this platform".to_string());
    None
}

#[cfg(target_os = "linux")]
fn try_wayland(errors: &mut Vec<String>) -> Option<Box<dyn Backend>> {
    match crate::backend::linux::wayland::WaylandBackend::try_new() {
        Ok(b) => Some(Box::new(b)),
        Err(e) => {
            tracing::warn!(backend = "wayland", error = %e, "backend unavailable");
            errors.push(format!("wayland: {e}"));
            None
        }
    }
}
#[cfg(not(target_os = "linux"))]
fn try_wayland(errors: &mut Vec<String>) -> Option<Box<dyn Backend>> {
    errors.push("wayland: Linux-only".to_string());
    None
}

#[cfg(target_os = "linux")]
fn try_portal(errors: &mut Vec<String>) -> Option<Box<dyn Backend>> {
    match crate::backend::linux::portal::PortalBackend::try_new() {
        Ok(b) => Some(Box::new(b)),
        Err(e) => {
            tracing::warn!(backend = "portal", error = %e, "backend unavailable");
            errors.push(format!("portal: {e}"));
            None
        }
    }
}
#[cfg(not(target_os = "linux"))]
fn try_portal(errors: &mut Vec<String>) -> Option<Box<dyn Backend>> {
    errors.push("portal: Linux-only".to_string());
    None
}

#[cfg(target_os = "linux")]
fn try_x11(errors: &mut Vec<String>) -> Option<Box<dyn Backend>> {
    match crate::backend::linux::x11::X11Backend::try_new() {
        Ok(b) => Some(Box::new(b)),
        Err(e) => {
            tracing::warn!(backend = "x11", error = %e, "backend unavailable");
            errors.push(format!("x11: {e}"));
            None
        }
    }
}
#[cfg(not(target_os = "linux"))]
fn try_x11(errors: &mut Vec<String>) -> Option<Box<dyn Backend>> {
    errors.push("x11: Linux-only".to_string());
    None
}

#[cfg(target_os = "windows")]
fn try_windows(errors: &mut Vec<String>) -> Option<Box<dyn Backend>> {
    match crate::backend::windows::WindowsBackend::try_new() {
        Ok(b) => Some(Box::new(b)),
        Err(e) => {
            errors.push(format!("windows: {e}"));
            None
        }
    }
}
#[cfg(not(target_os = "windows"))]
fn try_windows(errors: &mut Vec<String>) -> Option<Box<dyn Backend>> {
    errors.push("windows: Windows-only".to_string());
    None
}

#[cfg(target_os = "macos")]
fn try_macos(errors: &mut Vec<String>) -> Option<Box<dyn Backend>> {
    match crate::backend::macos::MacOsBackend::try_new() {
        Ok(b) => Some(Box::new(b)),
        Err(e) => {
            errors.push(format!("macos: {e}"));
            None
        }
    }
}
#[cfg(not(target_os = "macos"))]
fn try_macos(errors: &mut Vec<String>) -> Option<Box<dyn Backend>> {
    errors.push("macos: macOS-only".to_string());
    None
}
