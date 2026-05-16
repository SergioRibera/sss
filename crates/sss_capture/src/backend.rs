//! Internal capture-backend trait.

use image::RgbaImage;

use crate::error::Result;
use crate::geometry::{Point, Rect};
use crate::monitor::{Monitor, MonitorId};
use crate::options::CaptureOptions;
use crate::window::{Window, WindowId};

#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(target_os = "windows")]
pub(crate) mod windows;

pub(crate) mod compose;

pub(crate) trait Backend: Send {
    fn name(&self) -> &'static str;

    fn monitors(&self) -> Result<Vec<Monitor>>;

    fn windows(&self) -> Result<Vec<Window>>;

    fn capture_monitor(&self, id: MonitorId, opts: &CaptureOptions) -> Result<RgbaImage>;

    fn capture_window(&self, id: WindowId, opts: &CaptureOptions) -> Result<RgbaImage>;

    fn capture_all(&self, opts: &CaptureOptions) -> Result<RgbaImage>;

    fn capture_region(&self, region: Rect, opts: &CaptureOptions) -> Result<RgbaImage>;

    fn cursor_position(&self) -> Result<Point>;
}
