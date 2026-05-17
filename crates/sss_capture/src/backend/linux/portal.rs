//! `org.freedesktop.portal.Screenshot` fallback (GNOME, KDE under Wayland).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dbus::arg::{
    AppendAll, Iter, IterAppend, PropMap, ReadAll, RefArg, TypeMismatchError, Variant,
};
use dbus::blocking::Connection as DbusConnection;
use dbus::message::{MatchRule, SignalArgs};
use image::{ImageReader, RgbaImage};
use percent_encoding::percent_decode_str;

use crate::backend::Backend;
use crate::error::{CaptureError, Result};
use crate::geometry::{Point, Rect};
use crate::monitor::{Monitor, MonitorId};
use crate::options::CaptureOptions;
use crate::window::{Window, WindowId};

const BACKEND: &str = "wayland-portal";
// Shorter than dbus default: on compositors without a working portal screenshot
// backend the Response signal never arrives, so we time out and let the caller
// fall back to another backend.
const PORTAL_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Default)]
struct PortalResponse {
    status: u32,
    results: PropMap,
}

impl AppendAll for PortalResponse {
    fn append(&self, i: &mut IterAppend<'_>) {
        RefArg::append(&self.status, i);
        RefArg::append(&self.results, i);
    }
}

impl ReadAll for PortalResponse {
    fn read(i: &mut Iter<'_>) -> std::result::Result<Self, TypeMismatchError> {
        Ok(PortalResponse {
            status: i.read()?,
            results: i.read()?,
        })
    }
}

impl SignalArgs for PortalResponse {
    const NAME: &'static str = "Response";
    const INTERFACE: &'static str = "org.freedesktop.portal.Request";
}

pub(crate) struct PortalBackend {
    conn: Mutex<DbusConnection>,
}

impl PortalBackend {
    pub fn try_new() -> Result<Self> {
        let conn = DbusConnection::new_session()
            .map_err(|e| CaptureError::backend(BACKEND, format!("dbus session: {e}")))?;
        _ = conn
            .with_proxy(
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                Duration::from_secs(1),
            )
            .method_call::<(String,), _, _, _>(
                "org.freedesktop.DBus.Introspectable",
                "Introspect",
                (),
            );
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn screenshot(&self) -> Result<RgbaImage> {
        let conn = self.conn.lock().unwrap();
        let status: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));
        let uri: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

        // Install the signal handler before making the call to avoid a race.
        let match_rule = MatchRule::new_signal("org.freedesktop.portal.Request", "Response");
        let status_cb = status.clone();
        let uri_cb = uri.clone();
        let token = conn
            .add_match(match_rule, move |r: PortalResponse, _c, _m| {
                *status_cb.lock().unwrap() = Some(r.status);
                if let Some(u) = r.results.get("uri").and_then(|v| v.as_str()) {
                    *uri_cb.lock().unwrap() = u.to_string();
                }
                true
            })
            .map_err(|e| CaptureError::backend(BACKEND, format!("add_match: {e}")))?;

        let proxy = conn.with_proxy(
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            Duration::from_secs(5),
        );

        let mut options: PropMap = HashMap::new();
        options.insert(
            "handle_token".to_owned(),
            Variant(Box::new("sss_capture".to_owned())),
        );
        options.insert("modal".to_owned(), Variant(Box::new(true)));
        options.insert("interactive".to_owned(), Variant(Box::new(false)));

        let _: (dbus::Path<'_>,) = proxy
            .method_call(
                "org.freedesktop.portal.Screenshot",
                "Screenshot",
                ("", options),
            )
            .map_err(|e| CaptureError::backend(BACKEND, format!("Screenshot call: {e}")))?;

        let deadline = std::time::Instant::now() + PORTAL_TIMEOUT;
        loop {
            if std::time::Instant::now() > deadline {
                _ = conn.remove_match(token);
                return Err(CaptureError::Timeout(PORTAL_TIMEOUT));
            }
            conn.process(Duration::from_millis(250)).ok();
            if status.lock().unwrap().is_some() {
                break;
            }
        }
        _ = conn.remove_match(token);

        let status = status.lock().unwrap().unwrap_or(2);
        match status {
            0 => {}
            1 => return Err(CaptureError::Cancelled),
            other => {
                return Err(CaptureError::backend(
                    BACKEND,
                    format!("portal returned status {other}"),
                ))
            }
        }

        let uri = uri.lock().unwrap().clone();
        if uri.is_empty() {
            return Err(CaptureError::backend(BACKEND, "portal returned empty uri"));
        }
        let path = uri_to_path(&uri)?;
        let img = ImageReader::open(&path)
            .map_err(|e| CaptureError::backend(BACKEND, format!("open png: {e}")))?
            .decode()
            .map_err(|e| CaptureError::ImageConversion(format!("decode portal png: {e}")))?
            .to_rgba8();
        _ = std::fs::remove_file(&path);
        Ok(img)
    }
}

fn uri_to_path(uri: &str) -> Result<std::path::PathBuf> {
    let stripped = uri
        .strip_prefix("file://")
        .ok_or_else(|| CaptureError::backend(BACKEND, format!("uri is not file://: {uri}")))?;
    let decoded = percent_decode_str(stripped)
        .decode_utf8()
        .map_err(|e| CaptureError::backend(BACKEND, format!("uri utf8: {e}")))?;
    Ok(std::path::PathBuf::from(decoded.into_owned()))
}

impl Backend for PortalBackend {
    fn name(&self) -> &'static str {
        BACKEND
    }

    fn monitors(&self) -> Result<Vec<Monitor>> {
        // Portal does not enumerate displays; expose a synthetic primary.
        Ok(vec![Monitor {
            id: MonitorId(0),
            name: "Wayland (portal)".to_string(),
            bounds: Rect::from_xywh(0, 0, 0, 0),
            physical_size: (0, 0),
            scale_factor: 1.0,
            rotation: crate::geometry::Rotation::Normal,
            refresh_rate: None,
            is_primary: true,
        }])
    }

    fn windows(&self) -> Result<Vec<Window>> {
        Ok(Vec::new())
    }

    fn capture_monitor(&self, _id: MonitorId, _opts: &CaptureOptions) -> Result<RgbaImage> {
        self.screenshot()
    }

    fn capture_window(&self, id: WindowId, _opts: &CaptureOptions) -> Result<RgbaImage> {
        Err(CaptureError::WindowNotFound(id))
    }

    fn capture_all(&self, _opts: &CaptureOptions) -> Result<RgbaImage> {
        self.screenshot()
    }

    fn capture_region(&self, region: Rect, _opts: &CaptureOptions) -> Result<RgbaImage> {
        let full = self.screenshot()?;
        if region.x() < 0
            || region.y() < 0
            || region.x() + region.width() as i32 > full.width() as i32
            || region.y() + region.height() as i32 > full.height() as i32
        {
            return Err(CaptureError::RegionOutsideDesktop(region));
        }
        Ok(image::imageops::crop_imm(
            &full,
            region.x() as u32,
            region.y() as u32,
            region.width(),
            region.height(),
        )
        .to_image())
    }

    fn cursor_position(&self) -> Result<Point> {
        Err(CaptureError::CursorUnavailable(
            "the desktop portal does not expose pointer position".into(),
        ))
    }
}
