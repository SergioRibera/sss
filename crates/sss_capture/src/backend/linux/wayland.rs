//! Native Wayland capture backend using wlr-screencopy.

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::io::FromRawFd;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use image::RgbaImage;
use memmap2::MmapMut;
use wayland_client::globals::{registry_queue_init, Global, GlobalListContents};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::{
    delegate_noop, event_created_child, globals::GlobalList, Connection, Dispatch, EventQueue,
    Proxy, QueueHandle,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{
        self, ExtForeignToplevelListV1, EVT_TOPLEVEL_OPCODE as EXT_TOPLEVEL_OPCODE,
    },
};
use wayland_protocols::xdg::xdg_output::zv1::client::{
    zxdg_output_manager_v1::ZxdgOutputManagerV1,
    zxdg_output_v1::{self, ZxdgOutputV1},
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

use crate::backend::Backend;
use crate::error::{CaptureError, Result};
use crate::geometry::{Point, Rect, Rotation};
use crate::monitor::{Monitor, MonitorId};
use crate::options::CaptureOptions;
use crate::window::{Window, WindowId};

const BACKEND: &str = "wayland-wlr";
const FRAME_TIMEOUT: Duration = Duration::from_secs(5);

/// Dispatch wayland events with a wall-clock deadline; `Ok(false)` on timeout.
fn dispatch_until<S: 'static>(
    conn: &Connection,
    queue: &mut EventQueue<S>,
    state: &mut S,
    deadline: Instant,
) -> Result<bool> {
    use rustix::event::{poll, PollFd, PollFlags};

    let drained = queue
        .dispatch_pending(state)
        .map_err(|e| CaptureError::backend(BACKEND, format!("dispatch_pending: {e}")))?;
    if drained > 0 {
        return Ok(true);
    }

    if let Err(e) = queue.flush() {
        return Err(CaptureError::backend(BACKEND, format!("flush: {e}")));
    }

    let now = Instant::now();
    if now >= deadline {
        return Ok(false);
    }
    let guard = match conn.prepare_read() {
        Some(g) => g,
        None => {
            let n = queue
                .dispatch_pending(state)
                .map_err(|e| CaptureError::backend(BACKEND, format!("dispatch_pending: {e}")))?;
            return Ok(n > 0);
        }
    };
    let fd = guard.connection_fd();
    let mut fds = [PollFd::new(&fd, PollFlags::IN)];
    match poll(&mut fds, None) {
        Ok(0) => {
            return Ok(false);
        }
        Ok(_) => {
            guard
                .read()
                .map_err(|e| CaptureError::backend(BACKEND, format!("read events: {e}")))?;
        }
        Err(rustix::io::Errno::INTR) => {
            return Ok(false);
        }
        Err(e) => {
            return Err(CaptureError::backend(BACKEND, format!("poll: {e}")));
        }
    }

    let n = queue
        .dispatch_pending(state)
        .map_err(|e| CaptureError::backend(BACKEND, format!("dispatch_pending: {e}")))?;
    Ok(n > 0)
}

#[derive(Default, Clone)]
struct OutputInfo {
    wl_output: Option<WlOutput>,
    name: String,
    description: String,
    make: String,
    model: String,
    physical_x: i32,
    physical_y: i32,
    physical_width: i32,
    physical_height: i32,
    logical_x: i32,
    logical_y: i32,
    logical_width: i32,
    logical_height: i32,
    mode_width: i32,
    mode_height: i32,
    refresh_mhz: i32,
    scale: i32,
    transform: i32,
    done: bool,
}

#[derive(Default, Clone, Debug)]
struct ToplevelInfo {
    title: String,
    app_id: String,
    is_active: bool,
    is_minimized: bool,
    is_maximized: bool,
}

#[derive(Default)]
struct WlState {
    outputs: HashMap<u32, OutputInfo>,
    advertised_formats: Vec<(wl_shm::Format, u32, u32, u32)>,
    pending_format: Option<(wl_shm::Format, u32, u32, u32)>,
    pending_flags: u32,
    buffer_done: bool,
    frame_done: bool,
    frame_failed: bool,
    wlr_toplevels: HashMap<u32, ToplevelInfo>,
    ext_toplevels: HashMap<u32, ToplevelInfo>,
}

impl WlState {
    fn reset_frame(&mut self) {
        self.advertised_formats.clear();
        self.pending_format = None;
        self.pending_flags = 0;
        self.buffer_done = false;
        self.frame_done = false;
        self.frame_failed = false;
    }
}

pub(crate) struct WaylandBackend {
    inner: Mutex<Inner>,
}

struct Inner {
    conn: Connection,
    globals: GlobalList,
    screencopy_mgr: ZwlrScreencopyManagerV1,
    xdg_output_mgr: Option<ZxdgOutputManagerV1>,
    wlr_toplevel_mgr: Option<ZwlrForeignToplevelManagerV1>,
    ext_toplevel_list: Option<ExtForeignToplevelListV1>,
    shm: WlShm,
}

impl WaylandBackend {
    pub fn try_new() -> Result<Self> {
        let conn = Connection::connect_to_env().map_err(|e| {
            let msg = e.to_string();
            // Hint when the wayland crates are in dlopen mode and the system
            // libwayland-client.so.0 isn't on the dynamic linker's path.
            if msg.to_lowercase().contains("could not be loaded") {
                eprintln!(
                    "sss_capture[wayland]: failed to load libwayland-client.so.0 — \
                     install the system Wayland client library (e.g. \
                     `pacman -S wayland`, `apt install libwayland-client0`, or \
                     `nix-shell -p wayland`) and rebuild."
                );
            }
            CaptureError::backend(BACKEND, format!("cannot connect to compositor: {msg}"))
        })?;

        let (globals, mut event_queue) = registry_queue_init::<WlState>(&conn)
            .map_err(|e| CaptureError::backend(BACKEND, format!("registry init failed: {e}")))?;

        let qh = event_queue.handle();

        let screencopy_mgr = globals
            .bind::<ZwlrScreencopyManagerV1, _, _>(&qh, 1..=3, ())
            .map_err(|_| {
                CaptureError::unsupported(
                    BACKEND,
                    "compositor does not advertise zwlr_screencopy_manager_v1",
                )
            })?;
        let shm = globals
            .bind::<WlShm, _, _>(&qh, 1..=1, ())
            .map_err(|e| CaptureError::backend(BACKEND, format!("wl_shm bind failed: {e}")))?;
        let xdg_output_mgr = globals
            .bind::<ZxdgOutputManagerV1, _, _>(&qh, 1..=3, ())
            .ok();
        let wlr_toplevel_mgr = globals
            .bind::<ZwlrForeignToplevelManagerV1, _, _>(&qh, 1..=3, ())
            .ok();
        let ext_toplevel_list = globals
            .bind::<ExtForeignToplevelListV1, _, _>(&qh, 1..=1, ())
            .ok();

        let mut state = WlState::default();
        for Global {
            name,
            interface,
            version,
        } in globals.contents().clone_list()
        {
            if interface == WlOutput::interface().name {
                let v = version.min(4);
                let output: WlOutput = globals.registry().bind(name, v, &qh, ());
                let oid = output.id().protocol_id();
                let info = OutputInfo {
                    wl_output: Some(output),
                    ..Default::default()
                };
                state.outputs.insert(oid, info);
            }
        }

        event_queue.roundtrip(&mut state).map_err(|e| {
            CaptureError::backend(BACKEND, format!("initial roundtrip failed: {e}"))
        })?;

        if let Some(mgr) = xdg_output_mgr.as_ref() {
            let ids: Vec<u32> = state.outputs.keys().copied().collect();
            for oid in ids {
                if let Some(wl_output) = state.outputs.get(&oid).and_then(|i| i.wl_output.clone()) {
                    mgr.get_xdg_output(&wl_output, &qh, oid);
                }
            }
            event_queue.roundtrip(&mut state).map_err(|e| {
                CaptureError::backend(BACKEND, format!("xdg_output roundtrip failed: {e}"))
            })?;
        }

        let inner = Inner {
            conn,
            globals,
            screencopy_mgr,
            xdg_output_mgr,
            wlr_toplevel_mgr,
            ext_toplevel_list,
            shm,
        };

        Ok(Self {
            inner: Mutex::new(inner),
        })
    }

    fn refresh_state(&self, inner: &Inner) -> Result<(EventQueue<WlState>, WlState)> {
        tracing::info!("refresh_state: binding outputs on a fresh queue");
        let mut event_queue = inner.conn.new_event_queue::<WlState>();
        let qh = event_queue.handle();
        let mut state = WlState::default();

        let mut bound = 0;
        for Global {
            name,
            interface,
            version,
        } in inner.globals.contents().clone_list()
        {
            if interface == WlOutput::interface().name {
                let v = version.min(4);
                let output: WlOutput = inner.globals.registry().bind(name, v, &qh, ());
                let oid = output.id().protocol_id();
                let info = OutputInfo {
                    wl_output: Some(output),
                    ..Default::default()
                };
                state.outputs.insert(oid, info);
                bound += 1;
            }
        }
        tracing::info!("refresh_state: bound {bound} wl_output(s); roundtripping");
        event_queue
            .roundtrip(&mut state)
            .map_err(|e| CaptureError::backend(BACKEND, format!("output roundtrip: {e}")))?;
        tracing::info!("refresh_state: wl_output info received");
        if let Some(mgr) = inner.xdg_output_mgr.as_ref() {
            let ids: Vec<u32> = state.outputs.keys().copied().collect();
            for oid in ids {
                if let Some(wl_output) = state.outputs.get(&oid).and_then(|i| i.wl_output.clone()) {
                    mgr.get_xdg_output(&wl_output, &qh, oid);
                }
            }
            event_queue.roundtrip(&mut state).map_err(|e| {
                CaptureError::backend(BACKEND, format!("xdg_output roundtrip: {e}"))
            })?;
            tracing::info!("refresh_state: xdg_output info received");
        } else {
            tracing::info!("refresh_state: no xdg_output_manager_v1");
        }

        Ok((event_queue, state))
    }

    fn find_output(&self, state: &WlState, id: MonitorId) -> Result<WlOutput> {
        for info in state.outputs.values() {
            if compute_monitor_id(info) == id {
                return info
                    .wl_output
                    .clone()
                    .ok_or(CaptureError::MonitorNotFound(id));
            }
        }
        Err(CaptureError::MonitorNotFound(id))
    }

    fn do_capture(
        &self,
        frame: ZwlrScreencopyFrameV1,
        event_queue: &mut EventQueue<WlState>,
        state: &mut WlState,
    ) -> Result<RgbaImage> {
        state.reset_frame();
        tracing::info!("do_capture: waiting for buffer_done");
        eprintln!("sss_capture[wayland]: waiting for buffer_done…");

        // Hold the inner lock only long enough to clone the handles we need;
        // dispatch_until below can block, which must not block other callers.
        let (conn, shm) = {
            let inner = self.inner.lock().unwrap();
            (inner.conn.clone(), inner.shm.clone())
        };
        let _ = event_queue.flush();

        // Sending `copy` before `buffer_done` is a protocol error on strict
        // compositors (niri / Hyprland refuse).
        let deadline = Instant::now() + FRAME_TIMEOUT;
        loop {
            if state.buffer_done || state.frame_failed {
                break;
            }
            if Instant::now() >= deadline {
                eprintln!(
                    "sss_capture[wayland]: timeout waiting for buffer_done after {FRAME_TIMEOUT:?}",
                );
                return Err(CaptureError::Timeout(FRAME_TIMEOUT));
            }
            dispatch_until(&conn, event_queue, state, deadline)?;
        }
        if state.frame_failed {
            eprintln!("sss_capture[wayland]: compositor returned `failed`");
            return Err(CaptureError::backend(BACKEND, "compositor returned failed"));
        }
        if state.advertised_formats.is_empty() {
            eprintln!("sss_capture[wayland]: no wl_shm formats advertised");
            return Err(CaptureError::backend(
                BACKEND,
                "compositor advertised no wl_shm formats",
            ));
        }
        tracing::info!(
            "do_capture: buffer_done received with {} format(s)",
            state.advertised_formats.len()
        );

        let formats: Vec<wl_shm::Format> = state
            .advertised_formats
            .iter()
            .map(|(f, _, _, _)| *f)
            .collect();
        let chosen_fmt = pick_format(&formats).ok_or_else(|| {
            CaptureError::backend(BACKEND, "no supported wl_shm format among advertised")
        })?;
        let (fmt, width, height, stride) = state
            .advertised_formats
            .iter()
            .find(|(f, _, _, _)| *f == chosen_fmt)
            .copied()
            .expect("chosen format must be in the advertised set");
        tracing::info!("do_capture: chosen format={fmt:?} size={width}x{height} stride={stride}",);

        let size = (stride as usize) * (height as usize);
        let (mut file, mmap) = create_shm(size)?;
        let pool = shm.create_pool(file.as_fd(), size as i32, &event_queue.handle(), ());
        let buffer: WlBuffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            fmt,
            &event_queue.handle(),
            (),
        );

        tracing::info!("do_capture: sending copy request");
        frame.copy(&buffer);
        let _ = event_queue.flush();

        let deadline = Instant::now() + FRAME_TIMEOUT;
        loop {
            if state.frame_done || state.frame_failed {
                break;
            }
            if Instant::now() >= deadline {
                eprintln!(
                    "sss_capture[wayland]: timeout waiting for `ready` after {FRAME_TIMEOUT:?}"
                );
                return Err(CaptureError::Timeout(FRAME_TIMEOUT));
            }
            dispatch_until(&conn, event_queue, state, deadline)?;
        }
        if state.frame_failed {
            eprintln!("sss_capture[wayland]: copy returned `failed`");
            return Err(CaptureError::backend(BACKEND, "compositor returned failed"));
        }
        tracing::info!("do_capture: frame ready");

        let bytes = &mmap[..];
        let img = decode_frame(bytes, fmt, width, height, stride, state.pending_flags)?;

        buffer.destroy();
        pool.destroy();
        frame.destroy();
        let _ = file.flush();

        Ok(img)
    }
}

impl Backend for WaylandBackend {
    fn name(&self) -> &'static str {
        BACKEND
    }

    fn monitors(&self) -> Result<Vec<Monitor>> {
        let inner = self.inner.lock().unwrap();
        let (_, state) = self.refresh_state(&inner)?;
        let mut out: Vec<Monitor> = state.outputs.values().map(monitor_from_info).collect();

        // Wayland has no formal "primary" concept; mark the first output.
        if !out.iter().any(|m| m.is_primary) {
            if let Some(first) = out.first_mut() {
                first.is_primary = true;
            }
        }
        if out.is_empty() {
            return Err(CaptureError::NoMonitors);
        }
        Ok(out)
    }

    fn windows(&self) -> Result<Vec<Window>> {
        let inner = self.inner.lock().unwrap();
        let mut event_queue = inner.conn.new_event_queue::<WlState>();
        let _qh = event_queue.handle();
        let mut state = WlState::default();

        if let Some(mgr) = inner.wlr_toplevel_mgr.as_ref() {
            let _ = mgr;
        }
        if let Some(list) = inner.ext_toplevel_list.as_ref() {
            let _ = list;
        }

        for _ in 0..3 {
            event_queue
                .roundtrip(&mut state)
                .map_err(|e| CaptureError::backend(BACKEND, e.to_string()))?;
        }

        let mut windows = Vec::new();
        for (id, t) in state.wlr_toplevels.iter() {
            windows.push(Window {
                id: WindowId(*id as u64),
                title: t.title.clone(),
                app_name: t.app_id.clone(),
                bounds: Rect::default(),
                monitor: None,
                is_minimized: t.is_minimized,
                is_maximized: t.is_maximized,
                is_focused: t.is_active,
            });
        }
        for (id, t) in state.ext_toplevels.iter() {
            if windows.iter().any(|w| w.id.raw() == *id as u64) {
                continue;
            }
            windows.push(Window {
                id: WindowId(*id as u64),
                title: t.title.clone(),
                app_name: t.app_id.clone(),
                bounds: Rect::default(),
                monitor: None,
                is_minimized: t.is_minimized,
                is_maximized: t.is_maximized,
                is_focused: t.is_active,
            });
        }
        Ok(windows)
    }

    fn capture_monitor(&self, id: MonitorId, opts: &CaptureOptions) -> Result<RgbaImage> {
        tracing::info!("capture_monitor: id={id} show_cursor={}", opts.show_cursor);
        eprintln!("sss_capture[wayland]: capture_monitor {id}");
        // Drop the lock before do_capture, which re-acquires it briefly.
        let (frame, mut queue, mut state) = {
            let inner = self.inner.lock().unwrap();
            let (queue, state) = self.refresh_state(&inner)?;
            let output = self.find_output(&state, id)?;
            tracing::info!("capture_monitor: sending capture_output request");
            let frame = inner.screencopy_mgr.capture_output(
                opts.show_cursor as i32,
                &output,
                &queue.handle(),
                (),
            );
            (frame, queue, state)
        };
        let img = self.do_capture(frame, &mut queue, &mut state)?;
        tracing::info!("capture_monitor: completed ({} bytes)", img.as_raw().len());
        Ok(img)
    }

    fn capture_window(&self, id: WindowId, opts: &CaptureOptions) -> Result<RgbaImage> {
        let _ = (id, opts);
        Err(CaptureError::unsupported(
            BACKEND,
            "window capture requires per-toplevel bounds, which wlr-screencopy does not provide; \
             use the portal backend for this on GNOME/KDE",
        ))
    }

    fn capture_all(&self, opts: &CaptureOptions) -> Result<RgbaImage> {
        tracing::info!("capture_all: composing per-monitor captures");
        eprintln!("sss_capture[wayland]: capture_all start");
        let out = crate::backend::compose::all_monitors(self, opts);
        match &out {
            Ok(img) => eprintln!(
                "sss_capture[wayland]: capture_all done ({}x{})",
                img.width(),
                img.height()
            ),
            Err(e) => eprintln!("sss_capture[wayland]: capture_all failed: {e}"),
        }
        out
    }

    fn capture_region(&self, region: Rect, opts: &CaptureOptions) -> Result<RgbaImage> {
        if region.size.is_empty() {
            return Err(CaptureError::EmptyRegion(region));
        }
        // Fast path when the region fits inside exactly one output.
        let request = {
            let inner = self.inner.lock().unwrap();
            let (queue, state) = self.refresh_state(&inner)?;
            let monitors: Vec<Monitor> = state.outputs.values().map(monitor_from_info).collect();
            let mut single = None;
            for m in &monitors {
                if let Some(inter) = m.bounds.intersection(&region) {
                    if inter == region {
                        single = Some(m.clone());
                        break;
                    }
                }
            }
            match single {
                Some(m) => {
                    let output = self.find_output(&state, m.id)?;
                    let local = Rect::from_xywh(
                        region.x() - m.bounds.x(),
                        region.y() - m.bounds.y(),
                        region.width(),
                        region.height(),
                    );
                    let frame = inner.screencopy_mgr.capture_output_region(
                        opts.show_cursor as i32,
                        &output,
                        local.x(),
                        local.y(),
                        local.width() as i32,
                        local.height() as i32,
                        &queue.handle(),
                        (),
                    );
                    Some((frame, queue, state))
                }
                None => None,
            }
        };
        if let Some((frame, mut queue, mut state)) = request {
            return self.do_capture(frame, &mut queue, &mut state);
        }
        crate::backend::compose::region(self, region, opts)
    }

    fn cursor_position(&self) -> Result<Point> {
        Err(CaptureError::CursorUnavailable(
            "Wayland does not allow apps to read the global pointer position".into(),
        ))
    }
}

fn compute_monitor_id(info: &OutputInfo) -> MonitorId {
    // FNV-1a 64-bit over the connector name, with a protocol-id fallback.
    let seed = if !info.name.is_empty() {
        info.name.as_bytes()
    } else {
        b""
    };
    let mut h: u64 = 0xcbf29ce484222325;
    for b in seed {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    if h == 0xcbf29ce484222325 {
        if let Some(o) = info.wl_output.as_ref() {
            return MonitorId(o.id().protocol_id() as u64);
        }
    }
    MonitorId(h)
}

fn monitor_from_info(info: &OutputInfo) -> Monitor {
    let (w, h) = (
        if info.logical_width > 0 {
            info.logical_width as u32
        } else if info.mode_width > 0 {
            info.mode_width as u32
        } else {
            info.physical_width as u32
        },
        if info.logical_height > 0 {
            info.logical_height as u32
        } else if info.mode_height > 0 {
            info.mode_height as u32
        } else {
            info.physical_height as u32
        },
    );
    let (x, y) = if info.logical_width > 0 {
        (info.logical_x, info.logical_y)
    } else {
        (info.physical_x, info.physical_y)
    };
    let display_name = if !info.description.is_empty() {
        info.description.clone()
    } else if !info.model.is_empty() || !info.make.is_empty() {
        format!("{} {}", info.make, info.model).trim().to_string()
    } else {
        info.name.clone()
    };
    Monitor {
        id: compute_monitor_id(info),
        name: if display_name.is_empty() {
            "wayland-output".to_string()
        } else {
            display_name
        },
        bounds: Rect::from_xywh(x, y, w, h),
        physical_size: (
            info.mode_width.max(0) as u32,
            info.mode_height.max(0) as u32,
        ),
        scale_factor: if info.scale > 0 {
            info.scale as f32
        } else {
            1.0
        },
        rotation: transform_to_rotation(info.transform),
        refresh_rate: if info.refresh_mhz > 0 {
            Some(info.refresh_mhz as f32 / 1000.0)
        } else {
            None
        },
        is_primary: false,
    }
}

fn transform_to_rotation(t: i32) -> Rotation {
    match t {
        0 => Rotation::Normal,
        1 => Rotation::Rotate90,
        2 => Rotation::Rotate180,
        3 => Rotation::Rotate270,
        4 => Rotation::Flipped,
        5 => Rotation::Flipped90,
        6 => Rotation::Flipped180,
        7 => Rotation::Flipped270,
        _ => Rotation::Normal,
    }
}

fn create_shm(size: usize) -> Result<(File, MmapMut)> {
    let fd = create_memfd("sss_capture", size)?;
    // SAFETY: fd is freshly created and owned; FromRawFd consumes ownership.
    let file = unsafe { File::from_raw_fd(fd.into_raw_fd()) };
    let mmap = unsafe { MmapMut::map_mut(&file) }
        .map_err(|e| CaptureError::backend(BACKEND, format!("mmap: {e}")))?;
    Ok((file, mmap))
}

fn create_memfd(name: &str, size: usize) -> Result<OwnedFd> {
    use rustix::fs::MemfdFlags;
    let fd = rustix::fs::memfd_create(name, MemfdFlags::CLOEXEC)
        .map_err(|e| CaptureError::backend(BACKEND, format!("memfd_create: {e}")))?;
    rustix::fs::ftruncate(&fd, size as u64)
        .map_err(|e| CaptureError::backend(BACKEND, format!("ftruncate: {e}")))?;
    Ok(fd)
}

trait IntoRawFd {
    fn into_raw_fd(self) -> std::os::fd::RawFd;
}
impl IntoRawFd for OwnedFd {
    fn into_raw_fd(self) -> std::os::fd::RawFd {
        std::os::fd::IntoRawFd::into_raw_fd(self)
    }
}

fn decode_frame(
    bytes: &[u8],
    fmt: wl_shm::Format,
    width: u32,
    height: u32,
    stride: u32,
    flags: u32,
) -> Result<RgbaImage> {
    let y_invert = (flags & 1) != 0;
    let mut rgba = vec![0u8; (width * height * 4) as usize];

    let row = |y: u32| {
        let start = (y * stride) as usize;
        &bytes[start..start + (width * 4) as usize]
    };

    for y in 0..height {
        let src_y = if y_invert { height - 1 - y } else { y };
        let src = row(src_y);
        let dst = &mut rgba[(y * width * 4) as usize..((y + 1) * width * 4) as usize];
        match fmt {
            wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 => {
                // Little-endian [31:0] A:R:G:B in memory is B,G,R,A.
                for x in 0..width as usize {
                    let s = &src[x * 4..x * 4 + 4];
                    let d = &mut dst[x * 4..x * 4 + 4];
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = if matches!(fmt, wl_shm::Format::Argb8888) {
                        s[3]
                    } else {
                        255
                    };
                }
            }
            wl_shm::Format::Abgr8888 | wl_shm::Format::Xbgr8888 => {
                for x in 0..width as usize {
                    let s = &src[x * 4..x * 4 + 4];
                    let d = &mut dst[x * 4..x * 4 + 4];
                    d[0] = s[0];
                    d[1] = s[1];
                    d[2] = s[2];
                    d[3] = if matches!(fmt, wl_shm::Format::Abgr8888) {
                        s[3]
                    } else {
                        255
                    };
                }
            }
            wl_shm::Format::Rgba8888 | wl_shm::Format::Rgbx8888 => {
                for x in 0..width as usize {
                    let s = &src[x * 4..x * 4 + 4];
                    let d = &mut dst[x * 4..x * 4 + 4];
                    d[0] = s[3];
                    d[1] = s[2];
                    d[2] = s[1];
                    d[3] = if matches!(fmt, wl_shm::Format::Rgba8888) {
                        s[0]
                    } else {
                        255
                    };
                }
            }
            wl_shm::Format::Bgra8888 | wl_shm::Format::Bgrx8888 => {
                for x in 0..width as usize {
                    let s = &src[x * 4..x * 4 + 4];
                    let d = &mut dst[x * 4..x * 4 + 4];
                    d[0] = s[1];
                    d[1] = s[2];
                    d[2] = s[3];
                    d[3] = if matches!(fmt, wl_shm::Format::Bgra8888) {
                        s[0]
                    } else {
                        255
                    };
                }
            }
            other => {
                return Err(CaptureError::backend(
                    BACKEND,
                    format!("unsupported wl_shm format: {other:?}"),
                ));
            }
        }
    }

    RgbaImage::from_raw(width, height, rgba).ok_or_else(|| {
        CaptureError::ImageConversion(format!("buffer too small for {width}x{height}"))
    })
}

fn pick_format(advertised: &[wl_shm::Format]) -> Option<wl_shm::Format> {
    let order = [
        wl_shm::Format::Xrgb8888,
        wl_shm::Format::Argb8888,
        wl_shm::Format::Xbgr8888,
        wl_shm::Format::Abgr8888,
        wl_shm::Format::Rgbx8888,
        wl_shm::Format::Rgba8888,
        wl_shm::Format::Bgrx8888,
        wl_shm::Format::Bgra8888,
    ];
    order.iter().find(|f| advertised.contains(f)).copied()
}

impl Dispatch<WlRegistry, GlobalListContents> for WlState {
    fn event(
        _state: &mut Self,
        _proxy: &WlRegistry,
        _event: <WlRegistry as Proxy>::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlOutput, ()> for WlState {
    fn event(
        state: &mut Self,
        proxy: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let oid = proxy.id().protocol_id();
        let info = state.outputs.entry(oid).or_insert_with(|| OutputInfo {
            wl_output: Some(proxy.clone()),
            ..Default::default()
        });
        match event {
            wl_output::Event::Geometry {
                x,
                y,
                physical_width: _,
                physical_height: _,
                subpixel: _,
                make,
                model,
                transform,
            } => {
                info.physical_x = x;
                info.physical_y = y;
                info.make = make;
                info.model = model;
                info.transform = match transform {
                    wayland_client::WEnum::Value(t) => t as i32,
                    _ => 0,
                };
            }
            wl_output::Event::Mode {
                flags: _,
                width,
                height,
                refresh,
            } => {
                info.mode_width = width;
                info.mode_height = height;
                info.refresh_mhz = refresh;
                if info.physical_width == 0 {
                    info.physical_width = width;
                    info.physical_height = height;
                }
            }
            wl_output::Event::Scale { factor } => {
                info.scale = factor;
            }
            wl_output::Event::Name { name } => {
                info.name = name;
            }
            wl_output::Event::Description { description } => {
                info.description = description;
            }
            wl_output::Event::Done => {
                info.done = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<ZxdgOutputV1, u32> for WlState {
    fn event(
        state: &mut Self,
        _proxy: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        oid: &u32,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let info = state.outputs.entry(*oid).or_default();
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => {
                info.logical_x = x;
                info.logical_y = y;
            }
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                info.logical_width = width;
                info.logical_height = height;
            }
            zxdg_output_v1::Event::Name { name } if info.name.is_empty() => {
                info.name = name;
            }
            zxdg_output_v1::Event::Description { description } if info.description.is_empty() => {
                info.description = description;
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for WlState {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format: wayland_client::WEnum::Value(fmt),
                width,
                height,
                stride,
            } => {
                tracing::info!(
                    "screencopy: Buffer event fmt={fmt:?} {width}x{height} stride={stride}",
                );
                state.advertised_formats.push((fmt, width, height, stride));
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => {
                tracing::info!("screencopy: BufferDone");
                state.buffer_done = true;
            }
            zwlr_screencopy_frame_v1::Event::Flags {
                flags: wayland_client::WEnum::Value(f),
            } => {
                state.pending_flags = f.bits();
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                state.frame_done = true;
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                state.frame_failed = true;
            }
            zwlr_screencopy_frame_v1::Event::LinuxDmabuf { .. }
            | zwlr_screencopy_frame_v1::Event::Damage { .. } => {}
            _ => {}
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for WlState {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                state
                    .wlr_toplevels
                    .insert(toplevel.id().protocol_id(), ToplevelInfo::default());
            }
            zwlr_foreign_toplevel_manager_v1::Event::Finished => {}
            _ => {}
        }
    }

    event_created_child!(WlState, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE =>
            (ZwlrForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for WlState {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let id = handle.id().protocol_id();
        let t = state.wlr_toplevels.entry(id).or_default();
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => t.title = title,
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => t.app_id = app_id,
            zwlr_foreign_toplevel_handle_v1::Event::State { state: s } => {
                let chunks = s.chunks_exact(4);
                let states: Vec<u32> = chunks
                    .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                t.is_active = states.contains(&2);
                t.is_minimized = states.contains(&1);
                t.is_maximized = states.contains(&0);
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.wlr_toplevels.remove(&id);
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for WlState {
    fn event(
        state: &mut Self,
        _proxy: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state
                .ext_toplevels
                .insert(toplevel.id().protocol_id(), ToplevelInfo::default());
        }
    }

    event_created_child!(WlState, ExtForeignToplevelListV1, [
        EXT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for WlState {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let id = handle.id().protocol_id();
        let t = state.ext_toplevels.entry(id).or_default();
        match event {
            ext_foreign_toplevel_handle_v1::Event::Title { title } => t.title = title,
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => t.app_id = app_id,
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                state.ext_toplevels.remove(&id);
            }
            _ => {}
        }
    }
}

delegate_noop!(WlState: ignore WlShm);
delegate_noop!(WlState: ignore WlShmPool);
delegate_noop!(WlState: ignore WlBuffer);
delegate_noop!(WlState: ignore ZwlrScreencopyManagerV1);
delegate_noop!(WlState: ignore ZxdgOutputManagerV1);
