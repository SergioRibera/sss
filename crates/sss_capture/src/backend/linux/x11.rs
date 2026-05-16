//! X11 backend implemented on top of `x11rb`.

use std::sync::Mutex;

use image::RgbaImage;
use x11rb::connection::Connection;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    self, AtomEnum, ConnectionExt as _, GetGeometryReply, ImageFormat, ImageOrder, PropMode,
    Window as XWindow,
};
use x11rb::rust_connection::RustConnection;

use crate::backend::compose;
use crate::backend::Backend;
use crate::error::{CaptureError, Result};
use crate::geometry::{Point, Rect, Rotation};
use crate::monitor::{Monitor, MonitorId};
use crate::options::CaptureOptions;
use crate::window::{Window, WindowId};

const BACKEND: &str = "x11";

#[allow(dead_code)]
struct Atoms {
    net_client_list: xproto::Atom,
    net_client_list_stacking: xproto::Atom,
    net_wm_name: xproto::Atom,
    net_wm_state: xproto::Atom,
    net_wm_state_hidden: xproto::Atom,
    net_wm_state_maximized_vert: xproto::Atom,
    net_wm_state_maximized_horz: xproto::Atom,
    net_active_window: xproto::Atom,
    utf8_string: xproto::Atom,
    wm_class: xproto::Atom,
    wm_state: xproto::Atom,
}

pub(crate) struct X11Backend {
    conn: Mutex<RustConnection>,
    root: XWindow,
    atoms: Atoms,
}

impl X11Backend {
    pub fn try_new() -> Result<Self> {
        let (conn, screen_num) = RustConnection::connect(None).map_err(|e| {
            CaptureError::backend(BACKEND, format!("cannot connect to X server: {e}"))
        })?;
        let root = conn
            .setup()
            .roots
            .get(screen_num)
            .ok_or_else(|| CaptureError::backend(BACKEND, "X server has no screens"))?
            .root;

        let atoms = Atoms {
            net_client_list: intern_atom(&conn, b"_NET_CLIENT_LIST")?,
            net_client_list_stacking: intern_atom(&conn, b"_NET_CLIENT_LIST_STACKING")?,
            net_wm_name: intern_atom(&conn, b"_NET_WM_NAME")?,
            net_wm_state: intern_atom(&conn, b"_NET_WM_STATE")?,
            net_wm_state_hidden: intern_atom(&conn, b"_NET_WM_STATE_HIDDEN")?,
            net_wm_state_maximized_vert: intern_atom(&conn, b"_NET_WM_STATE_MAXIMIZED_VERT")?,
            net_wm_state_maximized_horz: intern_atom(&conn, b"_NET_WM_STATE_MAXIMIZED_HORZ")?,
            net_active_window: intern_atom(&conn, b"_NET_ACTIVE_WINDOW")?,
            utf8_string: intern_atom(&conn, b"UTF8_STRING")?,
            wm_class: intern_atom(&conn, b"WM_CLASS")?,
            wm_state: intern_atom(&conn, b"WM_STATE")?,
        };

        Ok(Self {
            conn: Mutex::new(conn),
            root,
            atoms,
        })
    }

    fn capture_drawable(
        &self,
        drawable: xproto::Drawable,
        x: i16,
        y: i16,
        w: u16,
        h: u16,
    ) -> Result<RgbaImage> {
        let conn = self.conn.lock().unwrap();
        let setup = conn.setup();
        let bitmap_order = setup.bitmap_format_bit_order;

        let reply = conn
            .get_image(ImageFormat::Z_PIXMAP, drawable, x, y, w, h, u32::MAX)
            .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?
            .reply()
            .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?;
        let depth = reply.depth;
        let bytes = reply.data;

        let pixmap_fmt = setup
            .pixmap_formats
            .iter()
            .find(|f| f.depth == depth)
            .ok_or_else(|| {
                CaptureError::backend(BACKEND, format!("no pixmap format for depth {depth}"))
            })?;
        let bpp = pixmap_fmt.bits_per_pixel as u32;
        decode_image(&bytes, w as u32, h as u32, depth, bpp, bitmap_order)
    }
}

impl Backend for X11Backend {
    fn name(&self) -> &'static str {
        BACKEND
    }

    fn monitors(&self) -> Result<Vec<Monitor>> {
        let conn = self.conn.lock().unwrap();
        let monitors_reply = conn
            .randr_get_monitors(self.root, true)
            .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?
            .reply();

        let mut out = Vec::new();
        match monitors_reply {
            Ok(reply) => {
                for m in reply.monitors {
                    let name =
                        atom_name(&conn, m.name).unwrap_or_else(|| format!("output-{}", m.name));
                    let (rotation, refresh) = rotation_and_refresh(&conn, &m.outputs);
                    let scale = guess_scale_factor(&name, m.width, m.width_in_millimeters);
                    out.push(Monitor {
                        id: MonitorId(m.name as u64),
                        name,
                        bounds: Rect::from_xywh(
                            m.x as i32,
                            m.y as i32,
                            m.width as u32,
                            m.height as u32,
                        ),
                        physical_size: (m.width as u32, m.height as u32),
                        scale_factor: scale,
                        rotation,
                        refresh_rate: refresh,
                        is_primary: m.primary,
                    });
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "RANDR 1.5 GetMonitors failed; falling back to GetScreenResources");
                out = monitors_via_screen_resources(&conn, self.root)?;
            }
        }

        if out.is_empty() {
            return Err(CaptureError::NoMonitors);
        }
        Ok(out)
    }

    fn windows(&self) -> Result<Vec<Window>> {
        let conn = self.conn.lock().unwrap();
        let xids = client_list(&conn, self.root, &self.atoms)?;
        let active = active_window(&conn, self.root, &self.atoms);
        let mut out = Vec::with_capacity(xids.len());
        for xid in xids {
            if let Ok(w) = describe_window(&conn, xid, &self.atoms, active) {
                out.push(w);
            }
        }
        Ok(out)
    }

    fn capture_monitor(&self, id: MonitorId, opts: &CaptureOptions) -> Result<RgbaImage> {
        let monitor = self
            .monitors()?
            .into_iter()
            .find(|m| m.id == id)
            .ok_or(CaptureError::MonitorNotFound(id))?;
        let bounds = monitor.bounds;
        let _ = opts;
        self.capture_drawable(
            self.root,
            bounds.x() as i16,
            bounds.y() as i16,
            bounds.width() as u16,
            bounds.height() as u16,
        )
    }

    fn capture_window(&self, id: WindowId, _opts: &CaptureOptions) -> Result<RgbaImage> {
        let xid = id.raw() as XWindow;
        let conn = self.conn.lock().unwrap();
        let geom = window_geometry(&conn, xid)?;
        drop(conn);
        self.capture_drawable(xid, 0, 0, geom.width, geom.height)
    }

    fn capture_all(&self, opts: &CaptureOptions) -> Result<RgbaImage> {
        let monitors = self.monitors()?;
        let bounds = Rect::bounding(&monitors.iter().map(|m| m.bounds).collect::<Vec<_>>())
            .ok_or(CaptureError::NoMonitors)?;
        let _ = opts;
        self.capture_drawable(
            self.root,
            bounds.x() as i16,
            bounds.y() as i16,
            bounds.width() as u16,
            bounds.height() as u16,
        )
    }

    fn capture_region(&self, region: Rect, opts: &CaptureOptions) -> Result<RgbaImage> {
        if region.size.is_empty() {
            return Err(CaptureError::EmptyRegion(region));
        }
        let _ = opts;
        self.capture_drawable(
            self.root,
            region.x() as i16,
            region.y() as i16,
            region.width() as u16,
            region.height() as u16,
        )
        .or_else(|_| compose::region(self, region, opts))
    }

    fn cursor_position(&self) -> Result<Point> {
        let conn = self.conn.lock().unwrap();
        let reply = conn
            .query_pointer(self.root)
            .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?
            .reply()
            .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?;
        Ok(Point::new(reply.root_x as i32, reply.root_y as i32))
    }
}

fn intern_atom(conn: &RustConnection, name: &[u8]) -> Result<xproto::Atom> {
    Ok(conn
        .intern_atom(false, name)
        .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?
        .reply()
        .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?
        .atom)
}

fn atom_name(conn: &RustConnection, atom: xproto::Atom) -> Option<String> {
    let reply = conn.get_atom_name(atom).ok()?.reply().ok()?;
    String::from_utf8(reply.name).ok()
}

fn window_geometry(conn: &RustConnection, w: XWindow) -> Result<GetGeometryReply> {
    conn.get_geometry(w)
        .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?
        .reply()
        .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))
}

fn rotation_and_refresh(
    conn: &RustConnection,
    outputs: &[randr::Output],
) -> (Rotation, Option<f32>) {
    for output in outputs {
        if let Ok(cookie) = conn.randr_get_output_info(*output, 0) {
            if let Ok(info) = cookie.reply() {
                if info.crtc != 0 {
                    if let Ok(crtc_cookie) = conn.randr_get_crtc_info(info.crtc, 0) {
                        if let Ok(crtc) = crtc_cookie.reply() {
                            let rotation = rotation_from_randr(crtc.rotation);
                            return (rotation, None);
                        }
                    }
                }
            }
        }
    }
    (Rotation::Normal, None)
}

fn rotation_from_randr(r: randr::Rotation) -> Rotation {
    use randr::Rotation as R;
    if r.contains(R::ROTATE0) {
        Rotation::Normal
    } else if r.contains(R::ROTATE90) {
        Rotation::Rotate90
    } else if r.contains(R::ROTATE180) {
        Rotation::Rotate180
    } else if r.contains(R::ROTATE270) {
        Rotation::Rotate270
    } else {
        Rotation::Normal
    }
}

fn guess_scale_factor(_name: &str, _width_px: u16, _width_mm: u32) -> f32 {
    1.0
}

fn monitors_via_screen_resources(conn: &RustConnection, root: XWindow) -> Result<Vec<Monitor>> {
    let res = conn
        .randr_get_screen_resources(root)
        .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?
        .reply()
        .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?;
    let mut out = Vec::new();

    let modes: std::collections::HashMap<u32, f32> = res
        .modes
        .iter()
        .map(|m| {
            let hz = if m.htotal != 0 && m.vtotal != 0 {
                m.dot_clock as f32 / (m.htotal as u32 * m.vtotal as u32) as f32
            } else {
                0.0
            };
            (m.id, hz)
        })
        .collect();

    for output in res.outputs {
        let info = match conn.randr_get_output_info(output, 0) {
            Ok(c) => c.reply().ok(),
            Err(_) => None,
        };
        let info = match info {
            Some(i) if i.crtc != 0 && i.connection == randr::Connection::CONNECTED => i,
            _ => continue,
        };
        let crtc = match conn.randr_get_crtc_info(info.crtc, 0) {
            Ok(c) => match c.reply() {
                Ok(r) => r,
                Err(_) => continue,
            },
            Err(_) => continue,
        };
        let name = String::from_utf8_lossy(&info.name).into_owned();
        let refresh = modes.get(&crtc.mode).copied();
        out.push(Monitor {
            id: MonitorId(output as u64),
            name,
            bounds: Rect::from_xywh(
                crtc.x as i32,
                crtc.y as i32,
                crtc.width as u32,
                crtc.height as u32,
            ),
            physical_size: (crtc.width as u32, crtc.height as u32),
            scale_factor: 1.0,
            rotation: rotation_from_randr(crtc.rotation),
            refresh_rate: refresh,
            is_primary: false,
        });
    }

    if let Ok(cookie) = conn.randr_get_output_primary(root) {
        if let Ok(p) = cookie.reply() {
            if let Some(m) = out.iter_mut().find(|m| m.id.raw() == p.output as u64) {
                m.is_primary = true;
            }
        }
    }

    Ok(out)
}

fn client_list(conn: &RustConnection, root: XWindow, atoms: &Atoms) -> Result<Vec<XWindow>> {
    let reply = conn
        .get_property(
            false,
            root,
            atoms.net_client_list_stacking,
            AtomEnum::WINDOW,
            0,
            1 << 16,
        )
        .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?
        .reply()
        .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?;
    let mut windows: Vec<XWindow> = reply.value32().map(|v| v.collect()).unwrap_or_default();
    if windows.is_empty() {
        let reply = conn
            .get_property(
                false,
                root,
                atoms.net_client_list,
                AtomEnum::WINDOW,
                0,
                1 << 16,
            )
            .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?
            .reply()
            .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?;
        windows = reply.value32().map(|v| v.collect()).unwrap_or_default();
    }
    Ok(windows)
}

fn active_window(conn: &RustConnection, root: XWindow, atoms: &Atoms) -> Option<XWindow> {
    let reply = conn
        .get_property(false, root, atoms.net_active_window, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    reply.value32().and_then(|mut v| v.next())
}

fn describe_window(
    conn: &RustConnection,
    xid: XWindow,
    atoms: &Atoms,
    active: Option<XWindow>,
) -> Result<Window> {
    let geom = window_geometry(conn, xid)?;
    let coords = conn
        .translate_coordinates(xid, geom.root, 0, 0)
        .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?
        .reply()
        .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?;

    let title = get_string_property(conn, xid, atoms.net_wm_name, atoms.utf8_string)
        .or_else(|| {
            get_string_property(conn, xid, AtomEnum::WM_NAME.into(), AtomEnum::STRING.into())
        })
        .unwrap_or_default();

    let app_name = get_wm_class(conn, xid, atoms.wm_class).unwrap_or_default();

    let states = get_atom_list_property(conn, xid, atoms.net_wm_state);
    let is_max_v = states.contains(&atoms.net_wm_state_maximized_vert);
    let is_max_h = states.contains(&atoms.net_wm_state_maximized_horz);
    let is_hidden = states.contains(&atoms.net_wm_state_hidden);

    Ok(Window {
        id: WindowId(xid as u64),
        title,
        app_name,
        bounds: Rect::from_xywh(
            coords.dst_x as i32,
            coords.dst_y as i32,
            geom.width as u32,
            geom.height as u32,
        ),
        monitor: None,
        is_minimized: is_hidden,
        is_maximized: is_max_v && is_max_h,
        is_focused: active == Some(xid),
    })
}

fn get_string_property(
    conn: &RustConnection,
    w: XWindow,
    property: xproto::Atom,
    type_: xproto::Atom,
) -> Option<String> {
    let reply = conn
        .get_property(false, w, property, type_, 0, 1 << 16)
        .ok()?
        .reply()
        .ok()?;
    if reply.value.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&reply.value).into_owned())
}

fn get_atom_list_property(
    conn: &RustConnection,
    w: XWindow,
    property: xproto::Atom,
) -> Vec<xproto::Atom> {
    if let Ok(cookie) = conn.get_property(false, w, property, AtomEnum::ATOM, 0, 1 << 10) {
        if let Ok(reply) = cookie.reply() {
            return reply.value32().map(|v| v.collect()).unwrap_or_default();
        }
    }
    Vec::new()
}

fn get_wm_class(conn: &RustConnection, w: XWindow, wm_class: xproto::Atom) -> Option<String> {
    let reply = conn
        .get_property(false, w, wm_class, AtomEnum::STRING, 0, 1 << 10)
        .ok()?
        .reply()
        .ok()?;
    // WM_CLASS is two NUL-terminated strings: instance + class; we return class.
    let mut parts = reply.value.split(|b| *b == 0).filter(|p| !p.is_empty());
    let _instance = parts.next();
    parts
        .next()
        .map(|c| String::from_utf8_lossy(c).into_owned())
}

fn decode_image(
    bytes: &[u8],
    width: u32,
    height: u32,
    depth: u8,
    bpp: u32,
    bit_order: ImageOrder,
) -> Result<RgbaImage> {
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let lsb = bit_order == ImageOrder::LSB_FIRST;
    let stride = (width * bpp / 8) as usize;

    for y in 0..height {
        let row = &bytes[y as usize * stride..(y as usize + 1) * stride];
        for x in 0..width {
            let idx = (x * bpp / 8) as usize;
            let out = ((y * width + x) * 4) as usize;
            let (r, g, b, a) = match (depth, bpp) {
                (1..=8, _) => {
                    let p = row[idx];
                    (p, p, p, 255)
                }
                (15..=16, _) => {
                    let pixel = if lsb {
                        u16::from_le_bytes([row[idx], row[idx + 1]])
                    } else {
                        u16::from_be_bytes([row[idx], row[idx + 1]])
                    };
                    let r5 = (pixel >> 11) & 0x1f;
                    let g6 = (pixel >> 5) & 0x3f;
                    let b5 = pixel & 0x1f;
                    let r = (r5 * 255 / 31) as u8;
                    let g = (g6 * 255 / 63) as u8;
                    let b = (b5 * 255 / 31) as u8;
                    (r, g, b, 255)
                }
                (_, bpp) if bpp >= 24 => {
                    // LSB servers store BGRA in memory; MSB stores RGBA.
                    if lsb {
                        (row[idx + 2], row[idx + 1], row[idx], 255)
                    } else {
                        (row[idx], row[idx + 1], row[idx + 2], 255)
                    }
                }
                _ => {
                    return Err(CaptureError::backend(
                        BACKEND,
                        format!("unsupported pixmap: depth={depth} bpp={bpp}"),
                    ));
                }
            };
            rgba[out] = r;
            rgba[out + 1] = g;
            rgba[out + 2] = b;
            rgba[out + 3] = a;
        }
    }

    RgbaImage::from_raw(width, height, rgba).ok_or_else(|| {
        CaptureError::ImageConversion(format!("decoded buffer too small for {width}x{height}",))
    })
}

#[allow(dead_code)]
fn _silence_propmode(_: PropMode) {}
