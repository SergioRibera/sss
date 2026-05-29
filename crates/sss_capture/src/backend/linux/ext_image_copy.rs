//! Native Wayland capture backend using ext-image-copy-capture-v1.
//!
//! Upstream replacement for the wlroots-only zwlr-screencopy protocol; the
//! protocol cosmic-comp / mutter (eventually) / kwin (eventually) all converge
//! on. See `protocols/staging/ext-image-copy-capture/ext-image-copy-capture-v1.xml`.

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
    delegate_noop, globals::GlobalList, Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
    ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
    ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options},
    ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
};
use wayland_protocols::xdg::xdg_output::zv1::client::{
    zxdg_output_manager_v1::ZxdgOutputManagerV1,
    zxdg_output_v1::{self, ZxdgOutputV1},
};

use crate::backend::Backend;
use crate::error::{CaptureError, Result};
use crate::geometry::{Point, Rect, Rotation};
use crate::monitor::{Monitor, MonitorId};
use crate::options::CaptureOptions;
use crate::window::{Window, WindowId};

const BACKEND: &str = "wayland-ext-image-copy";
const FRAME_TIMEOUT: Duration = Duration::from_secs(5);

fn dispatch_until(
    conn: &Connection,
    queue: &mut EventQueue<WlState>,
    state: &mut WlState,
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

    if Instant::now() >= deadline {
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
        Ok(0) => return Ok(false),
        Ok(_) => {
            guard
                .read()
                .map_err(|e| CaptureError::backend(BACKEND, format!("read events: {e}")))?;
        }
        Err(rustix::io::Errno::INTR) => return Ok(false),
        Err(e) => return Err(CaptureError::backend(BACKEND, format!("poll: {e}"))),
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
}

#[derive(Default)]
struct WlState {
    outputs: HashMap<u32, OutputInfo>,
    // Session constraints, populated by `shm_format` / `buffer_size` events
    // until the matching `done` arrives.
    session_width: u32,
    session_height: u32,
    advertised_shm_formats: Vec<wl_shm::Format>,
    session_done: bool,
    session_stopped: bool,
    // Per-frame state.
    frame_ready: bool,
    frame_failed: bool,
    frame_transform: u32,
}

impl WlState {
    fn reset_frame(&mut self) {
        self.frame_ready = false;
        self.frame_failed = false;
        self.frame_transform = 0;
    }
}

pub(crate) struct ExtImageCopyBackend {
    inner: Mutex<Inner>,
}

struct Inner {
    conn: Connection,
    globals: GlobalList,
    copy_mgr: ExtImageCopyCaptureManagerV1,
    output_source_mgr: ExtOutputImageCaptureSourceManagerV1,
    xdg_output_mgr: Option<ZxdgOutputManagerV1>,
    shm: WlShm,
}

impl ExtImageCopyBackend {
    pub fn try_new() -> Result<Self> {
        let conn = Connection::connect_to_env().map_err(|e| {
            CaptureError::backend(BACKEND, format!("cannot connect to compositor: {e}"))
        })?;

        let (globals, mut event_queue) = registry_queue_init::<WlState>(&conn)
            .map_err(|e| CaptureError::backend(BACKEND, format!("registry init failed: {e}")))?;

        let qh = event_queue.handle();

        let copy_mgr = globals
            .bind::<ExtImageCopyCaptureManagerV1, _, _>(&qh, 1..=1, ())
            .map_err(|_| {
                CaptureError::unsupported(
                    BACKEND,
                    "compositor does not advertise ext_image_copy_capture_manager_v1",
                )
            })?;
        let output_source_mgr = globals
            .bind::<ExtOutputImageCaptureSourceManagerV1, _, _>(&qh, 1..=1, ())
            .map_err(|_| {
                CaptureError::unsupported(
                    BACKEND,
                    "compositor does not advertise ext_output_image_capture_source_manager_v1",
                )
            })?;
        let shm = globals
            .bind::<WlShm, _, _>(&qh, 1..=1, ())
            .map_err(|e| CaptureError::backend(BACKEND, format!("wl_shm bind failed: {e}")))?;
        let xdg_output_mgr = globals
            .bind::<ZxdgOutputManagerV1, _, _>(&qh, 1..=3, ())
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
                state.outputs.insert(
                    oid,
                    OutputInfo {
                        wl_output: Some(output),
                        ..Default::default()
                    },
                );
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

        Ok(Self {
            inner: Mutex::new(Inner {
                conn,
                globals,
                copy_mgr,
                output_source_mgr,
                xdg_output_mgr,
                shm,
            }),
        })
    }

    fn refresh_state(&self, inner: &Inner) -> Result<(EventQueue<WlState>, WlState)> {
        let mut event_queue = inner.conn.new_event_queue::<WlState>();
        let qh = event_queue.handle();
        let mut state = WlState::default();
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
                state.outputs.insert(
                    oid,
                    OutputInfo {
                        wl_output: Some(output),
                        ..Default::default()
                    },
                );
            }
        }
        event_queue
            .roundtrip(&mut state)
            .map_err(|e| CaptureError::backend(BACKEND, format!("output roundtrip: {e}")))?;
        if let Some(mgr) = inner.xdg_output_mgr.as_ref() {
            let ids: Vec<u32> = state.outputs.keys().copied().collect();
            for oid in ids {
                if let Some(wl_output) = state.outputs.get(&oid).and_then(|i| i.wl_output.clone()) {
                    mgr.get_xdg_output(&wl_output, &qh, oid);
                }
            }
            event_queue
                .roundtrip(&mut state)
                .map_err(|e| CaptureError::backend(BACKEND, format!("xdg_output roundtrip: {e}")))?;
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

    fn capture_output(&self, output: WlOutput, opts: &CaptureOptions) -> Result<RgbaImage> {
        let (conn, copy_mgr, output_source_mgr, shm) = {
            let inner = self.inner.lock().unwrap();
            (
                inner.conn.clone(),
                inner.copy_mgr.clone(),
                inner.output_source_mgr.clone(),
                inner.shm.clone(),
            )
        };

        let mut event_queue = conn.new_event_queue::<WlState>();
        let qh = event_queue.handle();
        let mut state = WlState::default();

        let session_opts = if opts.show_cursor {
            Options::PaintCursors
        } else {
            Options::empty()
        };
        let source: ExtImageCaptureSourceV1 = output_source_mgr.create_source(&output, &qh, ());
        let session: ExtImageCopyCaptureSessionV1 =
            copy_mgr.create_session(&source, session_opts, &qh, ());
        _ = event_queue.flush();

        // Wait for the first `done` (constraints batch).
        let deadline = Instant::now() + FRAME_TIMEOUT;
        loop {
            if state.session_done || state.session_stopped {
                break;
            }
            if Instant::now() >= deadline {
                session.destroy();
                source.destroy();
                return Err(CaptureError::Timeout(FRAME_TIMEOUT));
            }
            dispatch_until(&conn, &mut event_queue, &mut state, deadline)?;
        }
        if state.session_stopped {
            session.destroy();
            source.destroy();
            return Err(CaptureError::backend(BACKEND, "session stopped"));
        }
        if state.advertised_shm_formats.is_empty() {
            session.destroy();
            source.destroy();
            return Err(CaptureError::backend(
                BACKEND,
                "compositor advertised no shm formats",
            ));
        }
        if state.session_width == 0 || state.session_height == 0 {
            session.destroy();
            source.destroy();
            return Err(CaptureError::backend(BACKEND, "session never sent buffer_size"));
        }

        let chosen_fmt = pick_format(&state.advertised_shm_formats).ok_or_else(|| {
            CaptureError::backend(BACKEND, "no supported wl_shm format among advertised")
        })?;
        let width = state.session_width;
        let height = state.session_height;
        let stride: u32 = width.saturating_mul(4);
        let size = (stride as usize) * (height as usize);

        let (mut file, mmap) = create_shm(size)?;
        let pool = shm.create_pool(file.as_fd(), size as i32, &qh, ());
        let buffer: WlBuffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            chosen_fmt,
            &qh,
            (),
        );

        state.reset_frame();
        let frame: ExtImageCopyCaptureFrameV1 = session.create_frame(&qh, ());
        frame.attach_buffer(&buffer);
        frame.damage_buffer(0, 0, width as i32, height as i32);
        frame.capture();
        _ = event_queue.flush();

        let deadline = Instant::now() + FRAME_TIMEOUT;
        loop {
            if state.frame_ready || state.frame_failed || state.session_stopped {
                break;
            }
            if Instant::now() >= deadline {
                frame.destroy();
                buffer.destroy();
                pool.destroy();
                session.destroy();
                source.destroy();
                return Err(CaptureError::Timeout(FRAME_TIMEOUT));
            }
            dispatch_until(&conn, &mut event_queue, &mut state, deadline)?;
        }
        if state.frame_failed || state.session_stopped {
            frame.destroy();
            buffer.destroy();
            pool.destroy();
            session.destroy();
            source.destroy();
            return Err(CaptureError::backend(BACKEND, "frame capture failed"));
        }

        let img = decode_frame(&mmap[..], chosen_fmt, width, height, stride)?;

        frame.destroy();
        buffer.destroy();
        pool.destroy();
        session.destroy();
        source.destroy();
        _ = file.flush();

        Ok(img)
    }
}

impl Backend for ExtImageCopyBackend {
    fn name(&self) -> &'static str {
        BACKEND
    }

    fn monitors(&self) -> Result<Vec<Monitor>> {
        let inner = self.inner.lock().unwrap();
        let (_, state) = self.refresh_state(&inner)?;
        let mut out: Vec<Monitor> = state.outputs.values().map(monitor_from_info).collect();
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
        Ok(Vec::new())
    }

    fn capture_monitor(&self, id: MonitorId, opts: &CaptureOptions) -> Result<RgbaImage> {
        let output = {
            let inner = self.inner.lock().unwrap();
            let (_, state) = self.refresh_state(&inner)?;
            self.find_output(&state, id)?
        };
        self.capture_output(output, opts)
    }

    fn capture_window(&self, _id: WindowId, _opts: &CaptureOptions) -> Result<RgbaImage> {
        Err(CaptureError::unsupported(
            BACKEND,
            "window capture requires ext_foreign_toplevel_image_capture_source_manager_v1, \
             not yet wired up; use the portal backend",
        ))
    }

    fn capture_all(&self, opts: &CaptureOptions) -> Result<RgbaImage> {
        crate::backend::compose::all_monitors(self, opts)
    }

    fn capture_region(&self, region: Rect, opts: &CaptureOptions) -> Result<RgbaImage> {
        if region.size.is_empty() {
            return Err(CaptureError::EmptyRegion(region));
        }
        // ext-image-copy-capture-v1 has no native region request; capture
        // the source's full output and crop client-side.
        crate::backend::compose::region(self, region, opts)
    }

    fn cursor_position(&self) -> Result<Point> {
        Err(CaptureError::CursorUnavailable(
            "Wayland does not allow apps to read the global pointer position".into(),
        ))
    }
}

fn compute_monitor_id(info: &OutputInfo) -> MonitorId {
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
) -> Result<RgbaImage> {
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let row = |y: u32| {
        let start = (y * stride) as usize;
        &bytes[start..start + (width * 4) as usize]
    };
    for y in 0..height {
        let src = row(y);
        let dst = &mut rgba[(y * width * 4) as usize..((y + 1) * width * 4) as usize];
        match fmt {
            wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 => {
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
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlOutput, ()> for WlState {
    fn event(
        state: &mut Self,
        proxy: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
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
                make,
                model,
                transform,
                ..
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
                width,
                height,
                refresh,
                ..
            } => {
                info.mode_width = width;
                info.mode_height = height;
                info.refresh_mhz = refresh;
                if info.physical_width == 0 {
                    info.physical_width = width;
                    info.physical_height = height;
                }
            }
            wl_output::Event::Scale { factor } => info.scale = factor,
            wl_output::Event::Name { name } => info.name = name,
            wl_output::Event::Description { description } => info.description = description,
            _ => {}
        }
    }
}

impl Dispatch<ZxdgOutputV1, u32> for WlState {
    fn event(
        state: &mut Self,
        _: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        oid: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
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
            zxdg_output_v1::Event::Name { name } if info.name.is_empty() => info.name = name,
            zxdg_output_v1::Event::Description { description } if info.description.is_empty() => {
                info.description = description;
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for WlState {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                state.session_width = width;
                state.session_height = height;
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat {
                format: wayland_client::WEnum::Value(fmt),
            } => {
                state.advertised_shm_formats.push(fmt);
            }
            ext_image_copy_capture_session_v1::Event::Done => state.session_done = true,
            ext_image_copy_capture_session_v1::Event::Stopped => state.session_stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for WlState {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_frame_v1::Event::Transform {
                transform: wayland_client::WEnum::Value(t),
            } => {
                state.frame_transform = t as u32;
            }
            ext_image_copy_capture_frame_v1::Event::Ready => state.frame_ready = true,
            ext_image_copy_capture_frame_v1::Event::Failed { .. } => state.frame_failed = true,
            _ => {}
        }
    }
}

delegate_noop!(WlState: ignore WlShm);
delegate_noop!(WlState: ignore WlShmPool);
delegate_noop!(WlState: ignore WlBuffer);
delegate_noop!(WlState: ignore ExtImageCopyCaptureManagerV1);
delegate_noop!(WlState: ignore ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(WlState: ignore ExtImageCaptureSourceV1);
delegate_noop!(WlState: ignore ZxdgOutputManagerV1);
