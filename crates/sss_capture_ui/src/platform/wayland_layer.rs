//! Wayland layer-shell driver: CPU-rendered overlay via wlr-layer-shell.

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::io::FromRawFd;
use std::time::{Duration, Instant};

use memmap2::MmapMut;
use rustix::fs::Timespec;
use sss_capture::Image as CapImage;
use sss_capture::{Monitor, Rect as MonitorRect};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_keyboard::{self, KeyState, WlKeyboard};
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_pointer::{self, ButtonState, WlPointer};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{delegate_noop, Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};

use crate::canvas::{Canvas, CanvasEvent};
use crate::geometry::FPoint;
use crate::mode::SelectorMode;
use crate::selector::{Outcome, PostAction, Selection, Selector, SelectorError};

struct ProbeState;

impl Dispatch<WlRegistry, GlobalListContents> for ProbeState {
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

pub(crate) fn is_available() -> bool {
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(_) => return false,
    };
    let (globals, _) = match registry_queue_init::<ProbeState>(&conn) {
        Ok(g) => g,
        Err(_) => return false,
    };
    globals
        .contents()
        .clone_list()
        .into_iter()
        .any(|g| g.interface == ZwlrLayerShellV1::interface().name)
}

pub(crate) fn run(sel: Selector) -> Result<Selection, SelectorError> {
    let Selector { config, capturer } = sel;

    // Layer-shell paints behind the overlay so we always capture eagerly.
    let initial = match capturer.capture_all_with(config.capture_opts) {
        Ok(img) => Some(img),
        Err(e) => {
            tracing::warn!(error = %e, "eager capture failed; layer-shell overlay will be black");
            eprintln!(
                "sss_capture_ui[layer-shell]: eager capture failed ({e}); \
                 overlay will start blank — capture retries on confirm."
            );
            None
        }
    };

    let monitors = capturer.monitors().map_err(SelectorError::Capture)?;

    let conn = Connection::connect_to_env()
        .map_err(|e| SelectorError::Backend(format!("wayland connect: {e}")))?;
    let (globals, mut event_queue) = registry_queue_init::<State>(&conn)
        .map_err(|e| SelectorError::Backend(format!("registry init: {e}")))?;
    let qh = event_queue.handle();

    let compositor: WlCompositor = globals
        .bind(&qh, 1..=6, ())
        .map_err(|_| SelectorError::Backend("compositor missing wl_compositor".into()))?;
    let shm: WlShm = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| SelectorError::Backend("compositor missing wl_shm".into()))?;
    let layer_shell: ZwlrLayerShellV1 = globals
        .bind(&qh, 1..=4, ())
        .map_err(|_| SelectorError::Backend("compositor missing zwlr_layer_shell_v1".into()))?;
    let seat: WlSeat = globals
        .bind(&qh, 1..=8, ())
        .map_err(|_| SelectorError::Backend("compositor missing wl_seat".into()))?;

    let runtime_mode = match config.mode {
        SelectorMode::AnyOf => SelectorMode::Area,
        m => m,
    };

    let snap_step = config.ui.snap_step;
    let current_color = config.ui.default_stroke_color;
    let current_width = config.ui.default_stroke_width;
    let current_fill = config.ui.default_fill;
    let mut canvas = Canvas::new();
    if current_fill.is_some() {
        canvas.set_fill_color(current_fill);
    }
    let mut state = State {
        running: true,
        outcome: None,
        action: PostAction {
            copy: false,
            save: false,
            save_path_hint: config.save_path_hint.clone(),
        },
        canvas,
        runtime_mode,
        config: config.clone(),
        background: initial.clone(),
        overlays: Vec::new(),
        output_infos: HashMap::new(),
        active_overlay: None,
        pointer_pos_local: FPoint::default(),
        pointer_pos_global: FPoint::default(),
        mods: ModState::default(),
        pipette_pending: false,
        magnifier_on: false,
        snap_on: false,
        picker: None,
        picker_drag: None,
        width_popup: None,
        snap_popup: None,
        snap_marker: None,
        snap_step,
        current_color,
        current_width,
        current_fill,
        color_hover_at: None,
        width_hover: false,
        radial: None,
        transform_drag: None,
        cursor_ctx: None,
        compositor: Some(compositor.clone()),
        pointer: None,
        qh: Some(qh.clone()),
    };

    for g in globals.contents().clone_list() {
        if g.interface == WlOutput::interface().name {
            let v = g.version.min(4);
            let output: WlOutput = globals.registry().bind(g.name, v, &qh, g.name);
            state
                .output_infos
                .insert(g.name, WlOutputInfo::empty(output));
        }
    }
    tracing::info!(
        "layer-shell: bound {} wl_output(s); roundtripping for geometry",
        state.output_infos.len()
    );
    event_queue
        .roundtrip(&mut state)
        .map_err(|e| SelectorError::Backend(format!("wl_output roundtrip: {e}")))?;

    // Match by wl_output position (sss_capture::Monitor::bounds uses the
    // same coordinate space). Matching by index breaks on niri where the
    // wl_output advertisement order differs from sss_capture's enumeration.
    for monitor in &monitors {
        let matched: Option<(u32, WlOutput)> = state
            .output_infos
            .iter()
            .find(|(_, info)| {
                info.done && info.x == monitor.bounds().x() && info.y == monitor.bounds().y()
            })
            .map(|(name, info)| (*name, info.wl_output.clone()));

        let (target_name, target_output) = match matched {
            Some((n, o)) => (Some(n), Some(o)),
            None => {
                tracing::warn!(
                    monitor = %monitor.name(),
                    bounds = %monitor.bounds(),
                    "layer-shell: no wl_output match by position; surface will be placed by compositor"
                );
                (None, None)
            }
        };

        let wl_surface = compositor.create_surface(&qh, ());
        let layer = layer_shell.get_layer_surface(
            &wl_surface,
            target_output.as_ref(),
            Layer::Overlay,
            "sss_capture_ui".to_string(),
            &qh,
            state.overlays.len() as u32,
        );
        layer.set_anchor(Anchor::Top | Anchor::Left | Anchor::Right | Anchor::Bottom);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.set_size(monitor.bounds().width(), monitor.bounds().height());
        wl_surface.commit();

        tracing::info!(
            monitor = %monitor.name(),
            bounds = %monitor.bounds(),
            wl_output_name = ?target_name,
            "layer-shell: surface created"
        );

        state.overlays.push(Overlay {
            wl_surface,
            layer,
            monitor: monitor.clone(),
            configured: false,
            size: (monitor.bounds().width(), monitor.bounds().height()),
            buffers: Vec::with_capacity(2),
            needs_redraw: true,
            pointer_inside: false,
        });
    }

    let pointer = seat.get_pointer(&qh, ());
    let keyboard = seat.get_keyboard(&qh, ());
    state.pointer = Some(pointer.clone());
    match super::cursor::CursorContext::new(&conn, shm.clone(), 24) {
        Ok(ctx) => state.cursor_ctx = Some(ctx),
        Err(e) => tracing::warn!(error = %e, "cursor theme load failed; using compositor default"),
    }

    while state.running {
        let _ = dispatch_until(
            &conn,
            &mut event_queue,
            &mut state,
            Duration::from_millis(50),
        )?;
        for i in 0..state.overlays.len() {
            if state.overlays[i].needs_redraw && state.overlays[i].configured {
                render_overlay(i, &shm, &qh, &mut state);
            }
        }
    }

    let capturer_arc = capturer.clone();
    let final_image = build_outcome_image(&state, capturer_arc.as_ref());
    let outcome = match state.outcome.take().unwrap_or(Outcome::Cancelled) {
        Outcome::Region { rect, .. } => Outcome::Region {
            rect,
            image: final_image,
        },
        Outcome::Monitor { monitor, rect, .. } => Outcome::Monitor {
            monitor,
            rect,
            image: final_image,
        },
        Outcome::Window { window, rect, .. } => Outcome::Window {
            window,
            rect,
            image: final_image,
        },
        Outcome::Cancelled => Outcome::Cancelled,
    };
    // Tear down surfaces and round-trip before dropping the connection so the
    // compositor actually drops the overlay frames before we hand control back.
    for ov in state.overlays.iter_mut() {
        for buf in ov.buffers.drain(..) {
            buf.wl_buffer.destroy();
            buf.pool.destroy();
            drop(buf.mmap);
        }
        ov.layer.destroy();
        ov.wl_surface.attach(None, 0, 0);
        ov.wl_surface.commit();
        ov.wl_surface.destroy();
    }
    pointer.release();
    keyboard.release();
    let _ = event_queue.flush();
    let _ = event_queue.roundtrip(&mut state);
    drop(event_queue);
    drop(conn);

    Ok(Selection {
        outcome,
        canvas: state.canvas,
        action: state.action,
    })
}

fn build_outcome_image(state: &State, _capturer: &sss_capture::Capturer) -> Option<CapImage> {
    let bg = state.background.as_ref()?;
    let rect = match &state.outcome {
        Some(Outcome::Region { rect, .. })
        | Some(Outcome::Monitor { rect, .. })
        | Some(Outcome::Window { rect, .. }) => *rect,
        _ => return None,
    };
    let monitors_bb = MonitorRect::bounding(
        &state
            .overlays
            .iter()
            .map(|o| o.monitor.bounds())
            .collect::<Vec<_>>(),
    )?;
    let local_x = (rect.x() - monitors_bb.x()).max(0) as u32;
    let local_y = (rect.y() - monitors_bb.y()).max(0) as u32;
    let mut cropped =
        image::imageops::crop_imm(bg.as_rgba(), local_x, local_y, rect.width(), rect.height())
            .to_image();
    crate::render::composite::flatten(&mut cropped, &state.canvas, (rect.x(), rect.y()));
    Some(CapImage::new(cropped))
}

fn dispatch_until(
    conn: &Connection,
    queue: &mut wayland_client::EventQueue<State>,
    state: &mut State,
    timeout: Duration,
) -> Result<bool, SelectorError> {
    use rustix::event::{poll, PollFd, PollFlags};
    let drained = queue
        .dispatch_pending(state)
        .map_err(|e| SelectorError::Backend(format!("dispatch_pending: {e}")))?;
    if drained > 0 {
        return Ok(true);
    }
    let _ = queue.flush();
    let guard = match conn.prepare_read() {
        Some(g) => g,
        None => {
            let n = queue
                .dispatch_pending(state)
                .map_err(|e| SelectorError::Backend(format!("dispatch_pending: {e}")))?;
            return Ok(n > 0);
        }
    };
    let fd = guard.connection_fd();
    let mut fds = [PollFd::new(&fd, PollFlags::IN)];
    match poll(&mut fds, Timespec::try_from(timeout).ok().as_ref()) {
        Ok(0) => return Ok(false),
        Ok(_) => {
            guard
                .read()
                .map_err(|e| SelectorError::Backend(format!("read events: {e}")))?;
        }
        Err(rustix::io::Errno::INTR) => return Ok(false),
        Err(e) => return Err(SelectorError::Backend(format!("poll: {e}"))),
    }
    let n = queue
        .dispatch_pending(state)
        .map_err(|e| SelectorError::Backend(format!("dispatch_pending: {e}")))?;
    Ok(n > 0)
}

pub(crate) struct State {
    running: bool,
    outcome: Option<Outcome>,
    action: PostAction,
    canvas: Canvas,
    runtime_mode: SelectorMode,
    config: crate::selector::Config,
    background: Option<CapImage>,
    overlays: Vec<Overlay>,
    /// wl_output info keyed by registry global name; used to match each
    /// sss_capture::Monitor to its WlOutput by position.
    output_infos: HashMap<u32, WlOutputInfo>,
    active_overlay: Option<usize>,
    pointer_pos_local: FPoint,
    pointer_pos_global: FPoint,
    mods: ModState,
    /// While true, the next left click samples a background pixel.
    pipette_pending: bool,
    magnifier_on: bool,
    snap_on: bool,
    picker: Option<HsvPicker>,
    picker_drag: Option<HsvHit>,
    width_popup: Option<WidthPopup>,
    snap_popup: Option<SnapPopup>,
    snap_marker: Option<FPoint>,
    snap_step: f32,
    /// Stroke colour kept across tool switches.
    current_color: crate::color::Color,
    current_width: f32,
    current_fill: Option<crate::color::Color>,
    color_hover_at: Option<u64>,
    width_hover: bool,
    radial: Option<RadialMenu>,
    transform_drag: Option<TransformDrag>,
    cursor_ctx: Option<super::cursor::CursorContext>,
    compositor: Option<WlCompositor>,
    pointer: Option<WlPointer>,
    qh: Option<QueueHandle<State>>,
}

struct WlOutputInfo {
    wl_output: WlOutput,
    x: i32,
    y: i32,
    mode_w: i32,
    mode_h: i32,
    scale: i32,
    transform: i32,
    /// True once `wl_output.done` arrived; geometry is safe to read.
    done: bool,
}

impl WlOutputInfo {
    fn empty(wl_output: WlOutput) -> Self {
        Self {
            wl_output,
            x: 0,
            y: 0,
            mode_w: 0,
            mode_h: 0,
            scale: 1,
            transform: 0,
            done: false,
        }
    }
}

struct Overlay {
    wl_surface: WlSurface,
    layer: ZwlrLayerSurfaceV1,
    monitor: Monitor,
    configured: bool,
    size: (u32, u32),
    /// Each `BusyFlag` flips on attach and off on `wl_buffer.release`.
    buffers: Vec<OverlayBuffer>,
    needs_redraw: bool,
    pointer_inside: bool,
}

type BusyFlag = std::sync::Arc<std::sync::atomic::AtomicBool>;

struct OverlayBuffer {
    mmap: MmapMut,
    wl_buffer: WlBuffer,
    pool: WlShmPool,
    busy: BusyFlag,
    size: usize,
}

#[derive(Default, Clone, Copy, Debug)]
struct ModState {
    ctrl: bool,
    shift: bool,
    alt: bool,
    meta: bool,
}

fn render_overlay(idx: usize, shm: &WlShm, qh: &QueueHandle<State>, state: &mut State) {
    use std::sync::atomic::Ordering;
    let (w, h) = state.overlays[idx].size;
    if w == 0 || h == 0 {
        return;
    }
    let stride = w as usize * 4;
    let size = stride * h as usize;

    state.overlays[idx].buffers.retain(|b| b.size == size);

    let buf_idx = state.overlays[idx]
        .buffers
        .iter()
        .position(|b| !b.busy.load(Ordering::Acquire));
    let buf_idx = match buf_idx {
        Some(i) => i,
        None => {
            // Cap buffer count so a stuck compositor doesn't grow unbounded.
            if state.overlays[idx].buffers.len() >= 3 {
                tracing::trace!(
                    idx,
                    "layer-shell: all 3 buffers still busy; skipping this redraw"
                );
                return;
            }
            let (file, mmap) = match shm_alloc(size) {
                Ok(x) => x,
                Err(e) => {
                    tracing::warn!(error = %e, "layer-shell: shm alloc failed");
                    return;
                }
            };
            let busy: BusyFlag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
            let wl_buffer = pool.create_buffer(
                0,
                w as i32,
                h as i32,
                stride as i32,
                wl_shm::Format::Xrgb8888,
                qh,
                busy.clone(),
            );
            // pool dup'd the fd
            let _ = file;
            state.overlays[idx].buffers.push(OverlayBuffer {
                mmap,
                wl_buffer,
                pool,
                busy,
                size,
            });
            state.overlays[idx].buffers.len() - 1
        }
    };

    // Pull the buffer out so paint can borrow &State alongside &mut mmap.
    let mut buf = state.overlays[idx].buffers.swap_remove(buf_idx);
    paint(idx, w, h, &mut buf.mmap, state);
    // Mark busy before attach+commit so the release race is harmless.
    buf.busy.store(true, Ordering::Release);
    let attached = buf.wl_buffer.clone();
    state.overlays[idx].buffers.push(buf);

    let surface = &state.overlays[idx].wl_surface;
    surface.attach(Some(&attached), 0, 0);
    surface.damage_buffer(0, 0, w as i32, h as i32);
    surface.commit();
    state.overlays[idx].needs_redraw = false;
}

fn shm_alloc(size: usize) -> Result<(File, MmapMut), String> {
    use rustix::fs::MemfdFlags;
    let fd = rustix::fs::memfd_create("sss_capture_ui_layer", MemfdFlags::CLOEXEC)
        .map_err(|e| format!("memfd_create: {e}"))?;
    rustix::fs::ftruncate(&fd, size as u64).map_err(|e| format!("ftruncate: {e}"))?;
    let file = unsafe { File::from_raw_fd(std::os::fd::IntoRawFd::into_raw_fd(fd)) };
    let mmap = unsafe { MmapMut::map_mut(&file) }.map_err(|e| format!("mmap: {e}"))?;
    Ok((file, mmap))
}

/// Paint XRGB8888 ([B,G,R,X] little-endian) into the mmap.
fn paint(idx: usize, w: u32, h: u32, mmap: &mut MmapMut, state: &State) {
    let mon_bounds = state.overlays[idx].monitor.bounds();
    let bytes = mmap.as_mut();

    let monitors_bb = MonitorRect::bounding(
        &state
            .overlays
            .iter()
            .map(|o| o.monitor.bounds())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();

    let mut rgba = image::RgbaImage::from_pixel(w, h, image::Rgba([0x10, 0x10, 0x10, 0xff]));
    if let Some(bg) = state.background.as_ref() {
        let bg = bg.as_rgba();
        let off_x = (mon_bounds.x() - monitors_bb.x()).max(0) as u32;
        let off_y = (mon_bounds.y() - monitors_bb.y()).max(0) as u32;
        for y in 0..h {
            let src_y = (off_y + y).min(bg.height().saturating_sub(1));
            for x in 0..w {
                let src_x = (off_x + x).min(bg.width().saturating_sub(1));
                rgba.put_pixel(x, y, *bg.get_pixel(src_x, src_y));
            }
        }
    }
    crate::render::composite::flatten_with_preview(
        &mut rgba,
        &state.canvas,
        (mon_bounds.x(), mon_bounds.y()),
    );

    let raw = rgba.as_raw();
    for i in 0..(w * h) as usize {
        let s = i * 4;
        bytes[s] = raw[s + 2];
        bytes[s + 1] = raw[s + 1];
        bytes[s + 2] = raw[s];
        bytes[s + 3] = 0xff;
    }

    // With no active region the whole monitor is dimmed so the overlay reads
    // as engaged immediately; once a region exists only the outside is dimmed.
    let dim = state.config.ui.background_dim;
    let region_local = state.canvas.region().map(|region| {
        let r_x = (region.x() - mon_bounds.x()).max(0).min(w as i32);
        let r_y = (region.y() - mon_bounds.y()).max(0).min(h as i32);
        let r_w = ((region.x() + region.width() as i32) - mon_bounds.x())
            .max(0)
            .min(w as i32)
            - r_x;
        let r_h = ((region.y() + region.height() as i32) - mon_bounds.y())
            .max(0)
            .min(h as i32)
            - r_y;
        (r_x, r_y, r_w, r_h)
    });
    let visible_region = region_local.filter(|(_, _, rw, rh)| *rw > 0 && *rh > 0);
    if dim > 0 {
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                if let Some((r_x, r_y, r_w, r_h)) = visible_region {
                    if x >= r_x && x < r_x + r_w && y >= r_y && y < r_y + r_h {
                        continue;
                    }
                }
                let d = (y as u32 * w * 4 + x as u32 * 4) as usize;
                bytes[d] = bytes[d].saturating_sub(dim);
                bytes[d + 1] = bytes[d + 1].saturating_sub(dim);
                bytes[d + 2] = bytes[d + 2].saturating_sub(dim);
            }
        }
    }
    if let Some((r_x, r_y, r_w, r_h)) = visible_region {
        let outline = state.config.ui.region_outline_color.to_rgb();
        for x in 0..r_w {
            paint_pixel(bytes, w, h, (r_x + x) as u32, r_y as u32, outline);
            paint_pixel(
                bytes,
                w,
                h,
                (r_x + x) as u32,
                (r_y + r_h - 1) as u32,
                outline,
            );
        }
        for y in 0..r_h {
            paint_pixel(bytes, w, h, r_x as u32, (r_y + y) as u32, outline);
            paint_pixel(
                bytes,
                w,
                h,
                (r_x + r_w - 1) as u32,
                (r_y + y) as u32,
                outline,
            );
        }
    }

    if state.snap_on {
        if let Some(region) = state.canvas.region() {
            let r_x = (region.x() - mon_bounds.x()).max(0);
            let r_y = (region.y() - mon_bounds.y()).max(0);
            let r_w = ((region.x() + region.width() as i32) - mon_bounds.x())
                .max(0)
                .min(w as i32)
                - r_x;
            let r_h = ((region.y() + region.height() as i32) - mon_bounds.y())
                .max(0)
                .min(h as i32)
                - r_y;
            if r_w > 0 && r_h > 0 {
                draw_snap_grid_clip(
                    bytes,
                    w,
                    h,
                    state.snap_step,
                    (r_x, r_y, r_w as u32, r_h as u32),
                    // The grid must anchor to the same global origin as
                    // snap_point or the dots drift from the snap targets.
                    (mon_bounds.x(), mon_bounds.y()),
                );
            }
        }
    }

    draw_selection_decor(bytes, w, h, idx, state);

    let main_layout = state.toolbar_layout_for(idx);
    let side_layout = state.side_toolbar_layout_for(idx);
    if let Some(layout) = main_layout.as_ref() {
        layout.draw(bytes, w, h, state);
    }
    if let Some(layout) = side_layout.as_ref() {
        layout.draw(bytes, w, h, state);
    }
    if main_layout.is_none() && state.overlays[idx].pointer_inside {
        draw_text(
            bytes,
            w,
            h,
            12,
            12,
            match state.runtime_mode {
                SelectorMode::Area | SelectorMode::AnyOf => "DRAG TO SELECT  ESC CANCEL",
                SelectorMode::Monitor => "CLICK A MONITOR  ESC CANCEL",
                SelectorMode::Window => "CLICK A WINDOW  ESC CANCEL",
            },
            [220, 220, 220],
        );
    }

    if let Some(picker) = state.picker.as_ref() {
        if picker.overlay_idx == idx {
            draw_hsv_picker(bytes, w, h, picker);
        }
    }

    // Drawn before the width popup so the slider sits on top if both stack.
    if let Some(r) = state.radial.as_ref() {
        if r.overlay_idx == idx {
            let palette = &state.config.palette.color_palette;
            let mon = state.overlays[idx].monitor.bounds();
            let lx = state.pointer_pos_global.x as i32 - mon.x();
            let ly = state.pointer_pos_global.y as i32 - mon.y();
            let hover_c = r.slot_color(palette.len(), lx, ly);
            let hover_w = r.slot_width(lx, ly, palette.len(), state.config.ui.radial_widths.len());
            draw_radial_menu(
                bytes,
                w,
                h,
                r,
                palette,
                &state.config.ui.radial_widths,
                hover_c,
                hover_w,
            );
        }
    }

    if let Some(wp) = state.width_popup.as_ref() {
        if wp.overlay_idx == idx {
            draw_width_popup(bytes, w, h, wp, state.active_tool_width());
        }
    }

    if let Some(sp) = state.snap_popup.as_ref() {
        if sp.overlay_idx == idx {
            draw_snap_popup(bytes, w, h, sp, state.snap_step);
        }
    }

    if state.snap_on {
        if let Some(p) = state.snap_marker {
            let mon = state.overlays[idx].monitor.bounds();
            let lx = p.x as i32 - mon.x();
            let ly = p.y as i32 - mon.y();
            for d in -5..=5 {
                paint_pixel(bytes, w, h, (lx + d) as u32, ly as u32, [255, 255, 255]);
                paint_pixel(bytes, w, h, lx as u32, (ly + d) as u32, [255, 255, 255]);
            }
            for d in -5..=5 {
                paint_pixel(bytes, w, h, (lx + d) as u32, (ly - 1) as u32, [0, 0, 0]);
                paint_pixel(bytes, w, h, (lx + d) as u32, (ly + 1) as u32, [0, 0, 0]);
                paint_pixel(bytes, w, h, (lx - 1) as u32, (ly + d) as u32, [0, 0, 0]);
                paint_pixel(bytes, w, h, (lx + 1) as u32, (ly + d) as u32, [0, 0, 0]);
            }
        }
    }

    draw_magnifier(bytes, w, h, idx, state);

    if state.overlays[idx].pointer_inside {
        let mut hints = Vec::<&str>::new();
        if state.pipette_pending {
            hints.push("PIPETTE: click to sample colour");
        }
        if state.snap_on {
            hints.push("SNAP ON");
        }
        if state.magnifier_on {
            hints.push("MAGNIFIER ON");
        }
        if !hints.is_empty() {
            draw_text(bytes, w, h, 12, 38, &hints.join("   "), [255, 200, 80]);
        }
    }
}

#[inline]
fn paint_pixel(bytes: &mut [u8], w: u32, h: u32, x: u32, y: u32, rgb: [u8; 3]) {
    if x >= w || y >= h {
        return;
    }
    let d = (y * w * 4 + x * 4) as usize;
    bytes[d] = rgb[2];
    bytes[d + 1] = rgb[1];
    bytes[d + 2] = rgb[0];
    bytes[d + 3] = 0xff;
}

fn draw_text(bytes: &mut [u8], w: u32, h: u32, x: u32, y: u32, text: &str, rgb: [u8; 3]) {
    draw_text_sized(bytes, w, h, x, y, text, rgb, 13.0);
}

/// Draw `text` at `(x, y)`, alpha-blending each glyph against the BGRX buffer.
fn draw_text_sized(
    bytes: &mut [u8],
    w: u32,
    h: u32,
    x: u32,
    y: u32,
    text: &str,
    rgb: [u8; 3],
    px: f32,
) {
    let ascent = super::font::ascent(px);
    let mut pen_x = x as f32;
    let baseline = y as f32 + ascent;
    for ch in text.chars() {
        let glyph = match super::font::glyph_for(ch, px) {
            Some(g) => g,
            None => {
                pen_x += super::font::measure(&ch.to_string(), px);
                continue;
            }
        };
        let gx0 = pen_x + glyph.bearing_x as f32;
        let gy0 = baseline + glyph.bearing_y as f32;
        for gy in 0..glyph.height {
            for gx in 0..glyph.width {
                let coverage = glyph.pixels[(gy * glyph.width + gx) as usize];
                if coverage == 0 {
                    continue;
                }
                let dx = (gx0 + gx as f32).round() as i32;
                let dy = (gy0 + gy as f32).round() as i32;
                if dx < 0 || dy < 0 || (dx as u32) >= w || (dy as u32) >= h {
                    continue;
                }
                let d = ((dy as u32) * w + (dx as u32)) as usize * 4;
                let a = coverage as u16;
                let inv = 255 - a;
                bytes[d] = ((rgb[2] as u16 * a + bytes[d] as u16 * inv) / 255) as u8;
                bytes[d + 1] = ((rgb[1] as u16 * a + bytes[d + 1] as u16 * inv) / 255) as u8;
                bytes[d + 2] = ((rgb[0] as u16 * a + bytes[d + 2] as u16 * inv) / 255) as u8;
                bytes[d + 3] = 0xff;
            }
        }
        pen_x += glyph.advance;
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
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

impl Dispatch<ZwlrLayerSurfaceV1, u32> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        idx: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let idx = *idx as usize;
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                proxy.ack_configure(serial);
                if idx < state.overlays.len() {
                    if width > 0 && height > 0 {
                        state.overlays[idx].size = (width, height);
                    }
                    state.overlays[idx].configured = true;
                    state.overlays[idx].needs_redraw = true;
                    tracing::info!(
                        idx,
                        size = ?(width, height),
                        "layer-shell: surface configured",
                    );
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {
                tracing::info!(idx, "layer-shell: surface closed by compositor");
                state.running = false;
                state.outcome = Some(Outcome::Cancelled);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                serial,
                surface,
                surface_x,
                surface_y,
            } => {
                if let Some(i) = state.overlays.iter().position(|o| o.wl_surface == surface) {
                    state.active_overlay = Some(i);
                    state.overlays[i].pointer_inside = true;
                    state.overlays[i].needs_redraw = true;
                    update_pointer(state, i, surface_x, surface_y);
                    // The compositor needs this serial on every set_cursor.
                    if let Some(ctx) = state.cursor_ctx.as_mut() {
                        ctx.enter_serial = serial;
                        ctx.invalidate();
                    }
                    refresh_cursor(state);
                }
            }
            wl_pointer::Event::Leave { surface, .. } => {
                if let Some(i) = state.overlays.iter().position(|o| o.wl_surface == surface) {
                    state.overlays[i].pointer_inside = false;
                    state.overlays[i].needs_redraw = true;
                }
                if state
                    .active_overlay
                    .map(|a| {
                        a == {
                            let i: usize = state
                                .overlays
                                .iter()
                                .position(|o| o.wl_surface == surface)
                                .unwrap_or(usize::MAX);
                            i
                        }
                    })
                    .unwrap_or(false)
                {
                    state.active_overlay = None;
                }
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                if let Some(i) = state.active_overlay {
                    update_pointer(state, i, surface_x, surface_y);
                    if let Some(hit) = state.picker_drag {
                        if let Some(p) = state.picker.as_ref() {
                            let mon = state.overlays[i].monitor.bounds();
                            let lx = state.pointer_pos_global.x as i32 - mon.x();
                            let ly = state.pointer_pos_global.y as i32 - mon.y();
                            let _ = p;
                            state.apply_picker_hit(hit, lx, ly);
                            mark_all_redraw(state);
                            return;
                        }
                    }
                    if state.width_popup.as_ref().is_some_and(|w| w.dragging) {
                        let mon = state.overlays[i].monitor.bounds();
                        let lx = state.pointer_pos_global.x as i32 - mon.x();
                        let ly = state.pointer_pos_global.y as i32 - mon.y();
                        state.apply_width_slider(lx, ly);
                        mark_all_redraw(state);
                        return;
                    }
                    if state.snap_popup.as_ref().is_some_and(|w| w.dragging) {
                        let mon = state.overlays[i].monitor.bounds();
                        let lx = state.pointer_pos_global.x as i32 - mon.x();
                        let ly = state.pointer_pos_global.y as i32 - mon.y();
                        state.apply_snap_slider(lx, ly);
                        mark_all_redraw(state);
                        return;
                    }
                    // Snap only the canvas-facing position; pointer_pos_global
                    // keeps the raw cursor so the system cursor still tracks it.
                    let (p, snap_hit) = if state.snap_on {
                        let snapped =
                            snap_point(&state.canvas, state.pointer_pos_global, state.snap_step);
                        let dx = snapped.x - state.pointer_pos_global.x;
                        let dy = snapped.y - state.pointer_pos_global.y;
                        let hit = if (dx * dx + dy * dy).sqrt() > 0.5 {
                            Some(snapped)
                        } else {
                            None
                        };
                        (snapped, hit)
                    } else {
                        (state.pointer_pos_global, None)
                    };
                    state.snap_marker = snap_hit;
                    state.canvas.handle(CanvasEvent::PointerMove(p));
                    refresh_cursor(state);
                    if state.transform_drag.is_some() {
                        state.apply_transform_drag();
                        mark_all_redraw(state);
                        return;
                    }
                    state.update_hover_popups();
                    // Hover-only motion is skipped here; a full SHM redraw on
                    // every motion event caused a visible flicker.
                    if state.canvas.is_drag_active()
                        || state.magnifier_on
                        || state.pipette_pending
                        || state.snap_marker.is_some()
                        || state.radial.is_some()
                    {
                        let _ = i;
                        mark_all_redraw(state);
                    }
                }
            }
            wl_pointer::Event::Button {
                button,
                state: btn_state,
                ..
            } => {
                // BTN_RIGHT (0x111): commit polygon, or toggle radial menu.
                if button == 0x111 {
                    if matches!(btn_state, WEnum::Value(ButtonState::Pressed)) {
                        if state.canvas.is_drawing_polygon() {
                            state.canvas.commit_polygon();
                            mark_all_redraw(state);
                        } else if state.radial.is_some() {
                            state.radial = None;
                            mark_all_redraw(state);
                        } else if let Some(i) = state.active_overlay {
                            let mon = state.overlays[i].monitor.bounds();
                            let lx = state.pointer_pos_global.x as i32 - mon.x();
                            let ly = state.pointer_pos_global.y as i32 - mon.y();
                            let palette_len = state.config.palette.color_palette.len();
                            let probe = RadialMenu {
                                origin: (0, 0),
                                overlay_idx: i,
                            };
                            let (_, _, mw, mh) = probe.outer_rect(palette_len);
                            let mon_w = mon.width() as i32;
                            let mon_h = mon.height() as i32;
                            let ox = (lx - mw as i32 / 2).clamp(8, (mon_w - mw as i32 - 8).max(8));
                            let oy = (ly + 8).clamp(8, (mon_h - mh as i32 - 8).max(8));
                            state.radial = Some(RadialMenu {
                                origin: (ox, oy),
                                overlay_idx: i,
                            });
                            mark_all_redraw(state);
                        }
                    }
                    return;
                }
                if button != 0x110 {
                    return;
                }
                let pressed = matches!(btn_state, WEnum::Value(ButtonState::Pressed));
                if !pressed {
                    state.picker_drag = None;
                    if let Some(wp) = state.width_popup.as_mut() {
                        wp.dragging = false;
                    }
                    if let Some(sp) = state.snap_popup.as_mut() {
                        sp.dragging = false;
                    }
                    if state.transform_drag.is_some() {
                        state.transform_drag = None;
                        state.canvas.snapshot_history();
                        mark_all_redraw(state);
                    }
                }
                if let Some(i) = state.active_overlay {
                    if pressed {
                        if state.pipette_pending {
                            sample_pipette(state);
                            state.pipette_pending = false;
                            mark_all_redraw(state);
                            return;
                        }
                        if state.radial.is_some() {
                            let mon = state.overlays[i].monitor.bounds();
                            let lx = state.pointer_pos_global.x as i32 - mon.x();
                            let ly = state.pointer_pos_global.y as i32 - mon.y();
                            let palette_len = state.config.palette.color_palette.len();
                            let widths_len = state.config.ui.radial_widths.len();
                            let r = state.radial.as_ref().unwrap();
                            let inside = r.contains(palette_len, lx, ly);
                            let color_slot = r.slot_color(palette_len, lx, ly);
                            let width_slot = r.slot_width(lx, ly, palette_len, widths_len);
                            if let Some(slot) = color_slot {
                                let c = state.config.palette.color_palette[slot];
                                state.apply_pick_color(c);
                                state.radial = None;
                            } else if let Some(slot) = width_slot {
                                let w = state.config.ui.radial_widths[slot];
                                state.update_current_width(w);
                                state.radial = None;
                            } else if !inside {
                                state.radial = None;
                            }
                            mark_all_redraw(state);
                            return;
                        }
                        if state.pointer_in_picker() {
                            if let Some((hit, lx, ly)) = state.hit_picker() {
                                state.apply_picker_hit(hit, lx, ly);
                                state.picker_drag = Some(hit);
                            }
                            if let Some(p) = state.picker.as_mut() {
                                p.pinned = true;
                            }
                            mark_all_redraw(state);
                            return;
                        }
                        if state.pointer_in_width_popup() {
                            let (lx, ly) = state.pointer_local(i);
                            state.apply_width_slider(lx, ly);
                            if let Some(wp) = state.width_popup.as_mut() {
                                wp.dragging = true;
                                wp.pinned = true;
                            }
                            mark_all_redraw(state);
                            return;
                        }
                        if state.pointer_in_snap_popup() {
                            let (lx, ly) = state.pointer_local(i);
                            if state.pointer_on_snap_track() {
                                state.apply_snap_slider(lx, ly);
                                if let Some(sp) = state.snap_popup.as_mut() {
                                    sp.dragging = true;
                                }
                            }
                            if let Some(sp) = state.snap_popup.as_mut() {
                                sp.pinned = true;
                            }
                            mark_all_redraw(state);
                            return;
                        }
                        if let Some((_layout, action)) = state.pointer_on_toolbar() {
                            state.apply_button(action);
                            mark_all_redraw(state);
                            return;
                        }
                        if let Some(action) = pointer_on_selection_button(state) {
                            state.apply_button(action);
                            mark_all_redraw(state);
                            return;
                        }
                        if let Some((kind, _shape_rect)) = pointer_on_gizmo_handle(state) {
                            if let Some(id) = state.canvas.selected() {
                                let shape =
                                    state.canvas.shapes().iter().find(|s| s.id == id).cloned();
                                if let Some(original) = shape {
                                    let bounds = original.bounds();
                                    let cx = bounds.x() as f32 + bounds.width() as f32 / 2.0;
                                    let cy = bounds.y() as f32 + bounds.height() as f32 / 2.0;
                                    let center = FPoint::new(cx, cy);
                                    let pp = if state.snap_on {
                                        snap_point_step(state.pointer_pos_global, state.snap_step)
                                    } else {
                                        state.pointer_pos_global
                                    };
                                    state.transform_drag = Some(match kind {
                                        GizmoHandle::Rotate => TransformDrag::Rotate {
                                            center,
                                            start_angle: (pp.y - center.y).atan2(pp.x - center.x),
                                            original: Box::new(original),
                                        },
                                        GizmoHandle::Scale => {
                                            let dx = pp.x - center.x;
                                            let dy = pp.y - center.y;
                                            let d = (dx * dx + dy * dy).sqrt().max(1.0);
                                            TransformDrag::Scale {
                                                anchor: center,
                                                start_dist: d,
                                                original: Box::new(original),
                                            }
                                        }
                                    });
                                }
                            }
                            mark_all_redraw(state);
                            return;
                        }
                    }
                    // Click in empty space dismisses any pinned popups.
                    if pressed {
                        if let Some(p) = state.picker.as_ref() {
                            if p.pinned {
                                state.picker = None;
                                state.picker_drag = None;
                                state.color_hover_at = None;
                            }
                        }
                        if let Some(p) = state.width_popup.as_ref() {
                            if p.pinned {
                                state.width_popup = None;
                            }
                        }
                        if let Some(p) = state.snap_popup.as_ref() {
                            if p.pinned {
                                state.snap_popup = None;
                            }
                        }
                    }
                    let click_p = if state.snap_on {
                        snap_point(&state.canvas, state.pointer_pos_global, state.snap_step)
                    } else {
                        state.pointer_pos_global
                    };
                    if pressed {
                        state.canvas.handle(CanvasEvent::PointerDown(click_p));
                    } else {
                        state.canvas.handle(CanvasEvent::PointerUp(click_p));
                    }
                    let _ = i;
                    mark_all_redraw(state);
                }
            }
            wl_pointer::Event::Axis { axis, value, .. } => {
                if !matches!(axis, WEnum::Value(wl_pointer::Axis::VerticalScroll)) {
                    return;
                }
                if !state.pointer_on_width_button() {
                    return;
                }
                let step = if state.mods.shift { 5.0 } else { 1.0 };
                // Positive value = scroll down, which decreases the width.
                let delta = if value > 0.0 { -step } else { step };
                state.step_active_tool_width(delta);
                mark_all_redraw(state);
            }
            _ => {}
        }
    }
}

fn refresh_cursor(state: &mut State) {
    let pointer_on_tb = state.pointer_on_any_toolbar_bg();
    let pointer_pos = state.pointer_pos_global;
    let mode = state.runtime_mode;
    let on_gizmo = pointer_on_gizmo_handle(state).is_some();
    let desired = super::cursor::desired_cursor_ext(
        &state.canvas,
        pointer_pos,
        pointer_on_tb,
        on_gizmo,
        mode,
    );
    let (Some(ctx), Some(comp), Some(ptr)) = (
        state.cursor_ctx.as_mut(),
        state.compositor.clone(),
        state.pointer.clone(),
    ) else {
        return;
    };
    let _ = (&comp, &ptr);
    if let Some(qh) = state.qh.as_ref() {
        ctx.apply(&ptr, &comp, qh, desired);
    }
}

fn update_pointer(state: &mut State, overlay_idx: usize, sx: f64, sy: f64) {
    let mon = state.overlays[overlay_idx].monitor.bounds();
    state.pointer_pos_local = FPoint::new(sx as f32, sy as f32);
    state.pointer_pos_global = FPoint::new(mon.x() as f32 + sx as f32, mon.y() as f32 + sy as f32);
}

// Linux evdev keycodes; we skip xkbcommon to avoid the linker dependency.
mod ev {
    pub const ESC: u32 = 1;
    pub const ENTER: u32 = 28;
    pub const KP_ENTER: u32 = 96;
    pub const BACKSPACE: u32 = 14;
    pub const DELETE: u32 = 111;
    pub const C: u32 = 46;
    pub const S: u32 = 31;
    pub const Z: u32 = 44;
    pub const Y: u32 = 21;
    pub const LCTRL: u32 = 29;
    pub const RCTRL: u32 = 97;
    pub const LSHIFT: u32 = 42;
    pub const RSHIFT: u32 = 54;
    pub const LALT: u32 = 56;
    pub const RALT: u32 = 100;
    pub const LMETA: u32 = 125;
    pub const RMETA: u32 = 126;
    pub const LBRACKET: u32 = 26;
    pub const RBRACKET: u32 = 27;
    pub const P: u32 = 25;
    pub const M: u32 = 50;
    pub const G: u32 = 34;
    pub const MINUS: u32 = 12;
    pub const EQUAL: u32 = 13;
    pub const COMMA: u32 = 51;
    pub const DOT: u32 = 52;
}

impl Dispatch<WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap { .. } => {}
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                state.mods.shift = (mods_depressed & 0x01) != 0;
                state.mods.ctrl = (mods_depressed & 0x04) != 0;
                state.mods.alt = (mods_depressed & 0x08) != 0;
                state.mods.meta = (mods_depressed & 0x40) != 0;
                state.canvas.set_constrain(state.mods.shift);
            }
            wl_keyboard::Event::Key {
                key,
                state: key_state,
                ..
            } => {
                let pressed = matches!(key_state, WEnum::Value(KeyState::Pressed));
                // Track modifier press/release here too — wl_keyboard.modifiers
                // can lag behind the keys event for a chord.
                match key {
                    ev::LCTRL | ev::RCTRL => state.mods.ctrl = pressed,
                    ev::LSHIFT | ev::RSHIFT => {
                        state.mods.shift = pressed;
                        state.canvas.set_constrain(pressed);
                    }
                    ev::LALT | ev::RALT => state.mods.alt = pressed,
                    ev::LMETA | ev::RMETA => state.mods.meta = pressed,
                    _ => {}
                }
                if !pressed {
                    return;
                }
                match key {
                    ev::ESC => {
                        if state.canvas.is_typing_text() {
                            state.canvas.handle(CanvasEvent::TextCancel);
                            mark_all_redraw(state);
                        } else if state.pipette_pending {
                            state.pipette_pending = false;
                            mark_all_redraw(state);
                        } else if state.picker.is_some() {
                            state.picker = None;
                            mark_all_redraw(state);
                        } else {
                            state.outcome = Some(Outcome::Cancelled);
                            state.running = false;
                        }
                    }
                    ev::ENTER | ev::KP_ENTER => {
                        if state.canvas.is_typing_text() {
                            state.canvas.handle(CanvasEvent::TextCommit);
                            mark_all_redraw(state);
                        } else if state.canvas.is_drawing_polygon() {
                            state.canvas.commit_polygon();
                            mark_all_redraw(state);
                        } else {
                            confirm(state);
                        }
                    }
                    ev::BACKSPACE if state.mods.ctrl && state.mods.shift => {
                        state.canvas.clear_shapes();
                        mark_all_redraw(state);
                    }
                    ev::BACKSPACE => state.canvas.handle(CanvasEvent::Delete),
                    ev::DELETE => state.canvas.handle(CanvasEvent::Delete),
                    ev::C if state.mods.ctrl => {
                        state.action.copy = true;
                        confirm(state);
                    }
                    ev::S if state.mods.ctrl => {
                        state.action.save = true;
                        confirm(state);
                    }
                    ev::Z if state.mods.ctrl => {
                        if state.mods.shift {
                            state.canvas.handle(CanvasEvent::Redo);
                        } else {
                            state.canvas.handle(CanvasEvent::Undo);
                        }
                        mark_all_redraw(state);
                    }
                    ev::Y if state.mods.ctrl => {
                        state.canvas.handle(CanvasEvent::Redo);
                        mark_all_redraw(state);
                    }
                    ev::RBRACKET if state.mods.ctrl => {
                        if state.mods.shift {
                            state.canvas.raise_to_top();
                        } else {
                            state.canvas.raise_selected();
                        }
                        mark_all_redraw(state);
                    }
                    ev::LBRACKET if state.mods.ctrl => {
                        if state.mods.shift {
                            state.canvas.lower_to_bottom();
                        } else {
                            state.canvas.lower_selected();
                        }
                        mark_all_redraw(state);
                    }
                    ev::P if !state.mods.ctrl => {
                        state.pipette_pending = !state.pipette_pending;
                        mark_all_redraw(state);
                    }
                    ev::M if !state.mods.ctrl => {
                        state.magnifier_on = !state.magnifier_on;
                        mark_all_redraw(state);
                    }
                    ev::G if !state.mods.ctrl => {
                        state.snap_on = !state.snap_on;
                        mark_all_redraw(state);
                    }
                    ev::EQUAL => {
                        let f = if state.mods.shift { 1.25 } else { 1.10 };
                        state.canvas.scale_selected(f);
                        mark_all_redraw(state);
                    }
                    ev::MINUS => {
                        let f = if state.mods.shift { 0.80 } else { 0.91 };
                        state.canvas.scale_selected(f);
                        mark_all_redraw(state);
                    }
                    ev::COMMA => {
                        let deg = if state.mods.shift { 45.0 } else { 5.0 };
                        state.canvas.rotate_selected(-(deg as f32).to_radians());
                        mark_all_redraw(state);
                    }
                    ev::DOT => {
                        let deg = if state.mods.shift { 45.0 } else { 5.0 };
                        state.canvas.rotate_selected((deg as f32).to_radians());
                        mark_all_redraw(state);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn mark_all_redraw(state: &mut State) {
    for o in &mut state.overlays {
        o.needs_redraw = true;
    }
}

fn confirm(state: &mut State) {
    let region = state.canvas.region();
    let outcome = match state.runtime_mode {
        SelectorMode::Monitor | SelectorMode::AnyOf if region.is_none() => {
            state.active_overlay.map(|i| Outcome::Monitor {
                monitor: state.overlays[i].monitor.id(),
                rect: state.overlays[i].monitor.bounds(),
                image: None,
            })
        }
        SelectorMode::Monitor => state.active_overlay.map(|i| Outcome::Monitor {
            monitor: state.overlays[i].monitor.id(),
            rect: state.overlays[i].monitor.bounds(),
            image: None,
        }),
        _ => region.map(|r| Outcome::Region {
            rect: r,
            image: None,
        }),
    };
    state.outcome = outcome.or(Some(Outcome::Cancelled));
    state.running = false;
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        _state: &mut Self,
        _: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            tracing::debug!(?caps, "wl_seat: capabilities");
        }
    }
}

impl Dispatch<WlSurface, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSurface,
        _: <WlSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: wayland_client::protocol::wl_output::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_client::protocol::wl_output::Event;
        let info = match state.output_infos.get_mut(name) {
            Some(i) => i,
            None => return,
        };
        match event {
            Event::Geometry {
                x, y, transform, ..
            } => {
                info.x = x;
                info.y = y;
                if let WEnum::Value(t) = transform {
                    info.transform = t as i32;
                }
            }
            Event::Mode { width, height, .. } => {
                info.mode_w = width;
                info.mode_h = height;
            }
            Event::Scale { factor } => {
                info.scale = factor;
            }
            Event::Done => {
                info.done = true;
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ignore WlShmPool);
impl Dispatch<WlBuffer, BusyFlag> for State {
    fn event(
        _: &mut Self,
        _: &WlBuffer,
        event: wayland_client::protocol::wl_buffer::Event,
        busy: &BusyFlag,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use std::sync::atomic::Ordering;
        if matches!(event, wayland_client::protocol::wl_buffer::Event::Release) {
            busy.store(false, Ordering::Release);
        }
    }
}
delegate_noop!(State: ignore ZwlrLayerShellV1);

#[allow(dead_code)]
fn _silence_map(_: HashMap<u32, ()>) {}

#[allow(dead_code)]
fn _silence_write(_: &mut dyn Write) {}

#[allow(dead_code)]
fn _silence_instant() -> Instant {
    Instant::now()
}

const TB_PAD_X: i32 = 8;
const TB_PAD_Y: i32 = 6;
const TB_BTN_W: i32 = 34;
const TB_BTN_H: i32 = 34;
const TB_GAP: i32 = 4;
const TB_SEP: i32 = 12;
const TB_GAP_FROM_REGION: i32 = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ButtonAction {
    SelectTool(usize),
    /// Pick a tool and switch fill mode on for closed shapes.
    SelectToolFilled(usize),
    ClearAll,
    TogglePicker,
    ToggleWidthPopup,
    ToggleSnapPopup,
    RaiseSelected,
    LowerSelected,
    ToggleSnap,
    ToggleMagnifier,
    TogglePipette,
    DeleteSelected,
    Undo,
    Redo,
    Cancel,
    Confirm,
    Copy,
    Save,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ToolbarIcon {
    Pointer,
    Brush,
    Line,
    Arrow,
    Rectangle,
    RectangleFilled,
    Ellipse,
    EllipseFilled,
    Blur,
    Eraser,
    Step,
    Text,
    Polygon,
    PolygonFilled,
    Undo,
    Redo,
    Cancel,
    Confirm,
    Copy,
    Save,
    ColorSwatch,
    Clear,
    Pipette,
    Snap,
    Magnifier,
    Raise,
    Lower,
    Trash,
    GizmoScale,
    GizmoRotate,
}

type Shape = (i32, i32, u32, u32);

pub(crate) struct ToolbarButton {
    /// Rect in monitor-local pixels.
    pub rect: Shape,
    pub action: ButtonAction,
    pub icon: Option<ToolbarIcon>,
    pub label: std::borrow::Cow<'static, str>,
    pub tint: Option<[u8; 3]>,
    pub active: bool,
}

pub(crate) struct ToolbarLayout {
    bg: Shape,
    buttons: Vec<ToolbarButton>,
}

impl ToolbarLayout {
    fn button_rect_for(&self, action: ButtonAction) -> Option<Shape> {
        self.buttons
            .iter()
            .find(|b| b.action == action)
            .map(|b| b.rect)
    }

    fn hit(&self, x: i32, y: i32) -> Option<ButtonAction> {
        for b in &self.buttons {
            let (rx, ry, rw, rh) = b.rect;
            if x >= rx && x < rx + rw as i32 && y >= ry && y < ry + rh as i32 {
                return Some(b.action);
            }
        }
        None
    }

    fn draw(&self, bytes: &mut [u8], w: u32, h: u32, state: &State) {
        let chrome = &state.config.ui.chrome;
        let panel = chrome.toolbar_bg.to_rgb();
        let border = chrome.toolbar_border.to_rgb();
        let fg = chrome.toolbar_fg.to_rgb();
        let active = chrome.button_active_bg.to_rgb();
        let active_border = chrome.button_active_border.to_rgb();
        let button = chrome.button_bg.to_rgb();
        let darken = |rgb: [u8; 3], by: u8| {
            [
                rgb[0].saturating_sub(by),
                rgb[1].saturating_sub(by),
                rgb[2].saturating_sub(by),
            ]
        };
        fill_rect_bytes(bytes, w, h, self.bg, panel);
        outline_rect(bytes, w, h, self.bg, border);

        for b in &self.buttons {
            let (rx, ry, rw, rh) = b.rect;
            let fill = if b.active {
                active
            } else {
                b.tint.unwrap_or(button)
            };
            fill_rect_bytes(bytes, w, h, (rx, ry, rw, rh), fill);
            outline_rect(
                bytes,
                w,
                h,
                (rx, ry, rw, rh),
                if b.active {
                    active_border
                } else {
                    darken(border, 10)
                },
            );
            if let Some(icon) = b.icon {
                let cx = rx + rw as i32 / 2;
                let cy = ry + rh as i32 / 2;
                draw_icon(bytes, w, h, cx, cy, icon, fg);
            } else if !b.label.is_empty() {
                let label = b.label.as_ref();
                let px = 13.0;
                let text_w = super::font::measure(label, px).ceil() as i32;
                let text_h = (px * 0.75) as i32;
                let tx = rx + (rw as i32 - text_w) / 2;
                let ty = ry + (rh as i32 - text_h) / 2;
                draw_text(
                    bytes,
                    w,
                    h,
                    tx.max(rx + 2) as u32,
                    ty.max(ry + 2) as u32,
                    label,
                    fg,
                );
            }
        }
    }
}

/// CPU-draw a small icon centred on `(cx, cy)`.
fn draw_icon(bytes: &mut [u8], w: u32, h: u32, cx: i32, cy: i32, kind: ToolbarIcon, rgb: [u8; 3]) {
    if let Some(raster) = super::icons::rasterise(kind, rgb) {
        blit_icon(bytes, w, h, cx, cy, raster);
        return;
    }
    draw_icon_fallback(bytes, w, h, cx, cy, kind, rgb);
}

fn blit_icon(
    bytes: &mut [u8],
    w: u32,
    h: u32,
    cx: i32,
    cy: i32,
    raster: &super::icons::RasterIcon,
) {
    let x0 = cx - raster.width as i32 / 2;
    let y0 = cy - raster.height as i32 / 2;
    for y in 0..raster.height as i32 {
        for x in 0..raster.width as i32 {
            let src = ((y as u32) * raster.width + x as u32) as usize * 4;
            let r = raster.rgba[src];
            let g = raster.rgba[src + 1];
            let b = raster.rgba[src + 2];
            let a = raster.rgba[src + 3];
            if a == 0 {
                continue;
            }
            let dx = (x0 + x) as u32;
            let dy = (y0 + y) as u32;
            if dx >= w || dy >= h {
                continue;
            }
            let d = (dy * w + dx) as usize * 4;
            let inv = 255 - a as u16;
            bytes[d] = ((b as u16 * a as u16 + bytes[d] as u16 * inv) / 255) as u8;
            bytes[d + 1] = ((g as u16 * a as u16 + bytes[d + 1] as u16 * inv) / 255) as u8;
            bytes[d + 2] = ((r as u16 * a as u16 + bytes[d + 2] as u16 * inv) / 255) as u8;
            bytes[d + 3] = 0xff;
        }
    }
}

fn draw_icon_fallback(
    bytes: &mut [u8],
    w: u32,
    h: u32,
    cx: i32,
    cy: i32,
    kind: ToolbarIcon,
    rgb: [u8; 3],
) {
    let s = 7;
    match kind {
        ToolbarIcon::Pointer => {
            for i in 0..=s {
                paint_pixel(bytes, w, h, (cx - s + i) as u32, (cy - s + i) as u32, rgb);
            }
            for i in 0..=s {
                paint_pixel(bytes, w, h, (cx - s) as u32, (cy - s + i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx - s + i) as u32, (cy - s) as u32, rgb);
            }
            for i in 0..=(s - 1) {
                paint_pixel(bytes, w, h, (cx + i / 2) as u32, (cy + i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + i / 2 + 1) as u32, (cy + i) as u32, rgb);
            }
        }
        ToolbarIcon::Brush => {
            for i in -s..=s {
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy + i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + i + 1) as u32, (cy + i) as u32, rgb);
            }
            for dy in -1..=1 {
                for dx in -1..=1 {
                    paint_pixel(bytes, w, h, (cx + s + dx) as u32, (cy + s + dy) as u32, rgb);
                }
            }
        }
        ToolbarIcon::Line => {
            for i in -s..=s {
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy + i) as u32, rgb);
            }
        }
        ToolbarIcon::Arrow => {
            for i in -s..=s {
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy + i) as u32, rgb);
            }
            for i in 0..=4 {
                paint_pixel(bytes, w, h, (cx + s - i) as u32, (cy + s) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + s) as u32, (cy + s - i) as u32, rgb);
            }
        }
        ToolbarIcon::Rectangle => {
            for x in -s..=s {
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy - s) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy + s) as u32, rgb);
            }
            for y in -s..=s {
                paint_pixel(bytes, w, h, (cx - s) as u32, (cy + y) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + s) as u32, (cy + y) as u32, rgb);
            }
        }
        ToolbarIcon::Ellipse => {
            let r = s as f32;
            for t in 0..64 {
                let a = (t as f32 / 64.0) * std::f32::consts::TAU;
                let x = cx + (r * a.cos()).round() as i32;
                let y = cy + (r * a.sin()).round() as i32;
                paint_pixel(bytes, w, h, x as u32, y as u32, rgb);
            }
        }
        ToolbarIcon::Blur => {
            for y in -s..=s {
                for x in -s..=s {
                    if ((x + s) / 2 + (y + s) / 2) % 2 == 0 {
                        paint_pixel(bytes, w, h, (cx + x) as u32, (cy + y) as u32, rgb);
                    }
                }
            }
        }
        ToolbarIcon::Eraser => {
            for i in -s..=s {
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy + i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy - i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + i + 1) as u32, (cy + i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy + i + 1) as u32, rgb);
            }
        }
        ToolbarIcon::Step => {
            let r = s as f32;
            for t in 0..64 {
                let a = (t as f32 / 64.0) * std::f32::consts::TAU;
                let x = cx + (r * a.cos()).round() as i32;
                let y = cy + (r * a.sin()).round() as i32;
                paint_pixel(bytes, w, h, x as u32, y as u32, rgb);
            }
            draw_text(bytes, w, h, (cx - 4) as u32, (cy - 6) as u32, "1", rgb);
        }
        ToolbarIcon::Text => {
            for x in -s..=s {
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy - s) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy - s + 1) as u32, rgb);
            }
            for y in -s..=s {
                paint_pixel(bytes, w, h, cx as u32, (cy + y) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + 1) as u32, (cy + y) as u32, rgb);
            }
        }
        ToolbarIcon::Polygon => {
            let r = s as f32;
            let mut prev = (cx, cy - s);
            for i in 1..=5 {
                let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::TAU / 5.0;
                let x = cx + (r * a.cos()).round() as i32;
                let y = cy + (r * a.sin()).round() as i32;
                draw_line_simple(bytes, w, h, prev, (x, y), rgb);
                prev = (x, y);
            }
        }
        ToolbarIcon::Undo => {
            let r = (s - 1) as f32;
            for t in 16..56 {
                let a = (t as f32 / 64.0) * std::f32::consts::TAU;
                let x = cx + (r * a.cos()).round() as i32;
                let y = cy + (r * a.sin()).round() as i32;
                paint_pixel(bytes, w, h, x as u32, y as u32, rgb);
            }
            for i in 0..=3 {
                paint_pixel(bytes, w, h, (cx - s + i) as u32, (cy - 1) as u32, rgb);
                paint_pixel(bytes, w, h, (cx - s + i) as u32, (cy + i - 1) as u32, rgb);
            }
        }
        ToolbarIcon::Redo => {
            let r = (s - 1) as f32;
            for t in 8..48 {
                let a = (t as f32 / 64.0) * std::f32::consts::TAU;
                let x = cx + (r * a.cos()).round() as i32;
                let y = cy + (r * a.sin()).round() as i32;
                paint_pixel(bytes, w, h, x as u32, y as u32, rgb);
            }
            for i in 0..=3 {
                paint_pixel(bytes, w, h, (cx + s - i) as u32, (cy - 1) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + s - i) as u32, (cy + i - 1) as u32, rgb);
            }
        }
        ToolbarIcon::Cancel => {
            for i in -(s - 1)..=(s - 1) {
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy + i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy - i) as u32, rgb);
            }
        }
        ToolbarIcon::Confirm => {
            for i in 0..s / 2 {
                paint_pixel(bytes, w, h, (cx - s + i + 2) as u32, (cy + i) as u32, rgb);
                paint_pixel(
                    bytes,
                    w,
                    h,
                    (cx - s + i + 2) as u32,
                    (cy + i + 1) as u32,
                    rgb,
                );
            }
            for i in 0..s {
                paint_pixel(
                    bytes,
                    w,
                    h,
                    (cx - 1 + i) as u32,
                    (cy + s / 2 - i) as u32,
                    rgb,
                );
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy + s / 2 - i) as u32, rgb);
            }
        }
        ToolbarIcon::Copy => {
            for x in -s..=(s - 3) {
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy - s + 3) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy + s) as u32, rgb);
            }
            for y in -s + 3..=s {
                paint_pixel(bytes, w, h, (cx - s) as u32, (cy + y) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + s - 3) as u32, (cy + y) as u32, rgb);
            }
            for x in -s + 3..=s {
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy - s) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy + s - 3) as u32, rgb);
            }
            for y in -s..=(s - 3) {
                paint_pixel(bytes, w, h, (cx - s + 3) as u32, (cy + y) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + s) as u32, (cy + y) as u32, rgb);
            }
        }
        ToolbarIcon::Save => {
            for x in -s..=s {
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy - s) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy + s) as u32, rgb);
            }
            for y in -s..=s {
                paint_pixel(bytes, w, h, (cx - s) as u32, (cy + y) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + s) as u32, (cy + y) as u32, rgb);
            }
            for x in -(s - 3)..=(s - 3) {
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy - s + 3) as u32, rgb);
            }
            for x in -(s - 3)..=(s - 3) {
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy + 1) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy + s - 2) as u32, rgb);
            }
            for y in 1..=(s - 2) {
                paint_pixel(bytes, w, h, (cx - (s - 3)) as u32, (cy + y) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + (s - 3)) as u32, (cy + y) as u32, rgb);
            }
        }
        ToolbarIcon::GizmoScale => {
            for i in 0..=s {
                paint_pixel(bytes, w, h, (cx - s + i) as u32, (cy - s + i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + s - i) as u32, (cy + s - i) as u32, rgb);
            }
            for d in 0..=3 {
                paint_pixel(bytes, w, h, (cx - s + d) as u32, (cy - s) as u32, rgb);
                paint_pixel(bytes, w, h, (cx - s) as u32, (cy - s + d) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + s - d) as u32, (cy + s) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + s) as u32, (cy + s - d) as u32, rgb);
            }
        }
        ToolbarIcon::GizmoRotate => {
            for theta in 0..280 {
                let a = (theta as f32 - 30.0).to_radians();
                let rx = cx + ((s - 1) as f32 * a.cos()).round() as i32;
                let ry = cy + ((s - 1) as f32 * a.sin()).round() as i32;
                paint_pixel(bytes, w, h, rx as u32, ry as u32, rgb);
            }
            // Arrow tip at the end of the arc (~250°).
            let tip = 250.0_f32.to_radians();
            let tx = cx + ((s - 1) as f32 * tip.cos()).round() as i32;
            let ty = cy + ((s - 1) as f32 * tip.sin()).round() as i32;
            for d in 0..=3 {
                paint_pixel(bytes, w, h, (tx + d) as u32, (ty - d) as u32, rgb);
                paint_pixel(bytes, w, h, (tx + d) as u32, ty as u32, rgb);
            }
        }
        ToolbarIcon::RectangleFilled
        | ToolbarIcon::EllipseFilled
        | ToolbarIcon::PolygonFilled
        | ToolbarIcon::ColorSwatch
        | ToolbarIcon::Clear
        | ToolbarIcon::Pipette
        | ToolbarIcon::Snap
        | ToolbarIcon::Magnifier
        | ToolbarIcon::Raise
        | ToolbarIcon::Lower
        | ToolbarIcon::Trash => {
            for d in 0..s {
                paint_pixel(bytes, w, h, (cx + d) as u32, (cy - s + d) as u32, rgb);
                paint_pixel(bytes, w, h, (cx - d) as u32, (cy - s + d) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + d) as u32, (cy + s - d) as u32, rgb);
                paint_pixel(bytes, w, h, (cx - d) as u32, (cy + s - d) as u32, rgb);
            }
        }
    }
}

/// `(px, py)` is inside the chip rect, the popup rect, or the narrow
/// vertical bridge joining them. The bounding union was too generous and
/// kept popups open over unrelated chips.
fn in_chip_popup_corridor(chip: Shape, popup: Shape, px: i32, py: i32, slack: i32) -> bool {
    if rect_contains(chip, px, py, slack) {
        return true;
    }
    if rect_contains(popup, px, py, slack) {
        return true;
    }
    let chip_bottom = chip.1 + chip.3 as i32;
    let popup_bottom = popup.1 + popup.3 as i32;
    let (by, bh) = if popup.1 >= chip_bottom {
        (chip_bottom, (popup.1 - chip_bottom).max(0))
    } else if chip.1 >= popup_bottom {
        (popup_bottom, (chip.1 - popup_bottom).max(0))
    } else {
        (0, 0)
    };
    if bh > 0 {
        rect_contains((chip.0, by, chip.2, bh as u32), px, py, slack)
    } else {
        false
    }
}

fn rect_contains(rect: Shape, px: i32, py: i32, slack: i32) -> bool {
    let (x, y, w, h) = rect;
    px >= x - slack && px < x + w as i32 + slack && py >= y - slack && py < y + h as i32 + slack
}

fn rects_overlap(a: Shape, b: Shape) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    !(ax + aw as i32 <= bx || bx + bw as i32 <= ax || ay + ah as i32 <= by || by + bh as i32 <= ay)
}

fn fill_rect_bytes(bytes: &mut [u8], w: u32, h: u32, rect: Shape, rgb: [u8; 3]) {
    let (rx, ry, rw, rh) = rect;
    for y in ry.max(0)..(ry + rh as i32).min(h as i32) {
        for x in rx.max(0)..(rx + rw as i32).min(w as i32) {
            paint_pixel(bytes, w, h, x as u32, y as u32, rgb);
        }
    }
}

fn outline_rect(bytes: &mut [u8], w: u32, h: u32, rect: Shape, rgb: [u8; 3]) {
    let (rx, ry, rw, rh) = rect;
    if rw == 0 || rh == 0 {
        return;
    }
    let x0 = rx.max(0).min(w as i32 - 1);
    let y0 = ry.max(0).min(h as i32 - 1);
    let x1 = (rx + rw as i32 - 1).max(0).min(w as i32 - 1);
    let y1 = (ry + rh as i32 - 1).max(0).min(h as i32 - 1);
    for x in x0..=x1 {
        paint_pixel(bytes, w, h, x as u32, y0 as u32, rgb);
        paint_pixel(bytes, w, h, x as u32, y1 as u32, rgb);
    }
    for y in y0..=y1 {
        paint_pixel(bytes, w, h, x0 as u32, y as u32, rgb);
        paint_pixel(bytes, w, h, x1 as u32, y as u32, rgb);
    }
}

impl State {
    pub(crate) fn toolbar_layout_for(&self, idx: usize) -> Option<ToolbarLayout> {
        if !self.config.toolbar {
            return None;
        }
        let mon = self.overlays[idx].monitor.bounds();

        let (anchor_rect, follow_region) = match (self.canvas.region(), self.runtime_mode) {
            (Some(r), SelectorMode::Area) | (Some(r), SelectorMode::AnyOf)
                if r.width() >= 2 && r.height() >= 2 =>
            {
                (r, true)
            }
            (_, SelectorMode::Monitor) | (_, SelectorMode::Window) => {
                if self.active_overlay != Some(idx) {
                    return None;
                }
                (mon, false)
            }
            _ => return None,
        };

        // For region-anchored toolbars, prefer the overlay with the largest
        // intersection so a selection spanning two monitors gets only one bar.
        if follow_region {
            if let Some(inter) = mon.intersection(&anchor_rect) {
                let mut best = (idx, inter.width() as u64 * inter.height() as u64);
                for (i, ov) in self.overlays.iter().enumerate() {
                    if i == idx {
                        continue;
                    }
                    if let Some(intr) = ov.monitor.bounds().intersection(&anchor_rect) {
                        let a = intr.width() as u64 * intr.height() as u64;
                        if a > best.1 {
                            best = (i, a);
                        }
                    }
                }
                if best.0 != idx {
                    return None;
                }
            } else {
                return None;
            }
        }

        let mut buttons: Vec<ToolbarButton> = Vec::new();

        let tools = &self.config.palette.tools;
        let active_disc = std::mem::discriminant(&self.canvas.active_tool);
        let fill_on = self.canvas.fill_mode();
        for (i, tool) in tools.iter().enumerate() {
            use crate::tool::Tool;
            let is_closed = matches!(
                tool,
                Tool::Rectangle(_) | Tool::Ellipse(_) | Tool::Polygon(_)
            );
            // Outlined chip is active iff selected and (not-closed or fill off).
            let outlined_active =
                std::mem::discriminant(tool) == active_disc && (!is_closed || !fill_on);
            buttons.push(ToolbarButton {
                rect: (0, 0, TB_BTN_W as u32, TB_BTN_H as u32),
                action: ButtonAction::SelectTool(i),
                icon: Some(tool_icon(tool)),
                label: std::borrow::Cow::Borrowed(""),
                tint: None,
                active: outlined_active,
            });
            if is_closed {
                let filled_active = std::mem::discriminant(tool) == active_disc && fill_on;
                buttons.push(ToolbarButton {
                    rect: (0, 0, TB_BTN_W as u32, TB_BTN_H as u32),
                    action: ButtonAction::SelectToolFilled(i),
                    icon: Some(filled_tool_icon(tool)),
                    label: std::borrow::Cow::Borrowed(""),
                    tint: None,
                    active: filled_active,
                });
            }
        }

        fn make_action(icon: ToolbarIcon, action: ButtonAction) -> ToolbarButton {
            ToolbarButton {
                rect: (0, 0, TB_BTN_W as u32, TB_BTN_H as u32),
                action,
                icon: Some(icon),
                label: std::borrow::Cow::Borrowed(""),
                tint: None,
                active: false,
            }
        }
        buttons.push(ToolbarButton {
            rect: (0, 0, TB_BTN_W as u32, TB_BTN_H as u32),
            action: ButtonAction::TogglePicker,
            icon: Some(ToolbarIcon::ColorSwatch),
            label: std::borrow::Cow::Borrowed(""),
            tint: Some([
                self.current_color.0[0],
                self.current_color.0[1],
                self.current_color.0[2],
            ]),
            active: self.picker.is_some() || self.color_hover_at.is_some(),
        });
        let w = self.current_width.round() as i32;
        buttons.push(ToolbarButton {
            rect: (0, 0, (TB_BTN_W + 4) as u32, TB_BTN_H as u32),
            action: ButtonAction::ToggleWidthPopup,
            icon: None,
            label: std::borrow::Cow::Owned(format!("{w}px")),
            tint: None,
            active: self.width_popup.is_some() || self.width_hover,
        });
        buttons.push(ToolbarButton {
            rect: (0, 0, TB_BTN_W as u32, TB_BTN_H as u32),
            action: ButtonAction::TogglePipette,
            icon: Some(ToolbarIcon::Pipette),
            label: std::borrow::Cow::Borrowed(""),
            tint: None,
            active: self.pipette_pending,
        });
        buttons.push(ToolbarButton {
            rect: (0, 0, TB_BTN_W as u32, TB_BTN_H as u32),
            action: ButtonAction::ToggleSnap,
            icon: Some(ToolbarIcon::Snap),
            label: std::borrow::Cow::Borrowed(""),
            tint: None,
            active: self.snap_on,
        });
        buttons.push(ToolbarButton {
            rect: (0, 0, (TB_BTN_W + 4) as u32, TB_BTN_H as u32),
            action: ButtonAction::ToggleSnapPopup,
            icon: None,
            label: std::borrow::Cow::Owned(format!("{}px", self.snap_step.round() as i32)),
            tint: None,
            active: self.snap_popup.is_some(),
        });
        buttons.push(ToolbarButton {
            rect: (0, 0, TB_BTN_W as u32, TB_BTN_H as u32),
            action: ButtonAction::ToggleMagnifier,
            icon: Some(ToolbarIcon::Magnifier),
            label: std::borrow::Cow::Borrowed(""),
            tint: None,
            active: self.magnifier_on,
        });
        let _ = make_action;

        let mut total_w: i32 = TB_PAD_X * 2;
        let mut prev_kind: Option<u8> = None;
        for b in &buttons {
            // 0 = tools, 1 = everything else, used to insert separators.
            let kind = match b.action {
                ButtonAction::SelectTool(_) => 0u8,
                _ => 1,
            };
            if prev_kind.is_some() && prev_kind != Some(kind) {
                total_w += TB_SEP;
            } else if prev_kind.is_some() {
                total_w += TB_GAP;
            }
            total_w += b.rect.2 as i32;
            prev_kind = Some(kind);
        }
        let total_h = TB_BTN_H + TB_PAD_Y * 2;

        let region_local_x = anchor_rect.x() - mon.x();
        let region_local_y = anchor_rect.y() - mon.y();
        let region_local_w = anchor_rect.width() as i32;
        let mon_w = mon.width() as i32;
        let mon_h = mon.height() as i32;

        let mut tb_x = region_local_x + (region_local_w - total_w) / 2;
        tb_x = tb_x.max(8).min(mon_w - total_w - 8).max(8);

        let above = region_local_y - total_h - TB_GAP_FROM_REGION;
        let below = region_local_y + anchor_rect.height() as i32 + TB_GAP_FROM_REGION;
        let mut tb_y = if above >= 8 {
            above
        } else if below + total_h <= mon_h - 8 {
            below
        } else {
            8
        };
        tb_y = tb_y.max(8).min(mon_h - total_h - 8).max(8);

        let mut cursor_x = tb_x + TB_PAD_X;
        let cursor_y = tb_y + TB_PAD_Y;
        let mut prev_kind: Option<u8> = None;
        for b in buttons.iter_mut() {
            let kind = match b.action {
                ButtonAction::SelectTool(_) => 0u8,
                _ => 1,
            };
            if let Some(prev) = prev_kind {
                cursor_x += if prev != kind { TB_SEP } else { TB_GAP };
            }
            b.rect.0 = cursor_x;
            b.rect.1 = cursor_y;
            cursor_x += b.rect.2 as i32;
            prev_kind = Some(kind);
        }

        Some(ToolbarLayout {
            bg: (tb_x, tb_y, total_w as u32, total_h as u32),
            buttons,
        })
    }

    pub(crate) fn side_toolbar_layout_for(&self, idx: usize) -> Option<ToolbarLayout> {
        if !self.config.toolbar {
            return None;
        }
        let mon = self.overlays[idx].monitor.bounds();
        let (anchor_rect, follow_region) = match (self.canvas.region(), self.runtime_mode) {
            (Some(r), SelectorMode::Area) | (Some(r), SelectorMode::AnyOf)
                if r.width() >= 2 && r.height() >= 2 =>
            {
                (r, true)
            }
            (_, SelectorMode::Monitor) | (_, SelectorMode::Window) => {
                if self.active_overlay != Some(idx) {
                    return None;
                }
                (mon, false)
            }
            _ => return None,
        };

        if follow_region {
            if let Some(inter) = mon.intersection(&anchor_rect) {
                let mut best = (idx, inter.width() as u64 * inter.height() as u64);
                for (i, ov) in self.overlays.iter().enumerate() {
                    if i == idx {
                        continue;
                    }
                    if let Some(intr) = ov.monitor.bounds().intersection(&anchor_rect) {
                        let a = intr.width() as u64 * intr.height() as u64;
                        if a > best.1 {
                            best = (i, a);
                        }
                    }
                }
                if best.0 != idx {
                    return None;
                }
            } else {
                return None;
            }
        }

        fn make_action(icon: ToolbarIcon, action: ButtonAction) -> ToolbarButton {
            ToolbarButton {
                rect: (0, 0, TB_BTN_W as u32, TB_BTN_H as u32),
                action,
                icon: Some(icon),
                label: std::borrow::Cow::Borrowed(""),
                tint: None,
                active: false,
            }
        }
        let mut buttons: Vec<ToolbarButton> = vec![
            make_action(ToolbarIcon::Undo, ButtonAction::Undo),
            make_action(ToolbarIcon::Redo, ButtonAction::Redo),
            ToolbarButton {
                rect: (0, 0, TB_BTN_W as u32, TB_BTN_H as u32),
                action: ButtonAction::ClearAll,
                icon: Some(ToolbarIcon::Clear),
                label: std::borrow::Cow::Borrowed(""),
                tint: None,
                active: false,
            },
            make_action(ToolbarIcon::Cancel, ButtonAction::Cancel),
            make_action(ToolbarIcon::Confirm, ButtonAction::Confirm),
        ];
        if self.config.show_copy {
            buttons.push(make_action(ToolbarIcon::Copy, ButtonAction::Copy));
        }
        if self.config.show_save {
            buttons.push(make_action(ToolbarIcon::Save, ButtonAction::Save));
        }

        let total_h = TB_PAD_Y * 2
            + buttons.len() as i32 * TB_BTN_H
            + (buttons.len() as i32 - 1).max(0) * TB_GAP;
        let total_w = TB_PAD_X * 2 + TB_BTN_W;

        let region_local_x = anchor_rect.x() - mon.x();
        let region_local_y = anchor_rect.y() - mon.y();
        let region_local_h = anchor_rect.height() as i32;
        let mon_w = mon.width() as i32;
        let mon_h = mon.height() as i32;

        let mut tb_y = region_local_y + (region_local_h - total_h) / 2;
        tb_y = tb_y.max(8).min(mon_h - total_h - 8).max(8);

        // Use the farther of the region edge and the main toolbar edge so
        // the side bar always clears the main one.
        let main_bg = self.toolbar_layout_for(idx).map(|l| l.bg);
        let main_right = main_bg
            .as_ref()
            .map(|(x, _, w, _)| x + *w as i32)
            .unwrap_or(0);
        let main_left = main_bg.as_ref().map(|(x, _, _, _)| *x).unwrap_or(mon_w);
        let region_right = region_local_x + anchor_rect.width() as i32;
        let region_left = region_local_x;
        let right = region_right.max(main_right) + TB_GAP_FROM_REGION;
        let left = region_left.min(main_left) - total_w - TB_GAP_FROM_REGION;
        let mut tb_x = if right + total_w <= mon_w - 8 {
            right
        } else if left >= 8 {
            left
        } else {
            mon_w - total_w - 8
        };
        tb_x = tb_x.max(8).min(mon_w - total_w - 8).max(8);

        // Stack vertically when the side bar would overlap the main bar.
        if let Some(main) = main_bg {
            let side_rect = (tb_x, tb_y, total_w as u32, total_h as u32);
            if rects_overlap(side_rect, main) {
                let main_top = main.1;
                let main_bot = main.1 + main.3 as i32;
                let above_room = main_top - 8;
                let below_room = mon_h - main_bot - 8;
                if below_room >= total_h && below_room >= above_room {
                    tb_y = (main_bot + 4).min(mon_h - total_h - 8);
                } else {
                    tb_y = (main_top - total_h - 4).max(8);
                }
            }
        }

        let cursor_x = tb_x + TB_PAD_X;
        let mut cursor_y = tb_y + TB_PAD_Y;
        for b in buttons.iter_mut() {
            b.rect.0 = cursor_x;
            b.rect.1 = cursor_y;
            cursor_y += b.rect.3 as i32 + TB_GAP;
        }

        Some(ToolbarLayout {
            bg: (tb_x, tb_y, total_w as u32, total_h as u32),
            buttons,
        })
    }

    fn pointer_local(&self, idx: usize) -> (i32, i32) {
        let mon = self.overlays[idx].monitor.bounds();
        (
            (self.pointer_pos_global.x as i32) - mon.x(),
            (self.pointer_pos_global.y as i32) - mon.y(),
        )
    }

    fn pointer_on_toolbar(&self) -> Option<(ToolbarLayout, ButtonAction)> {
        let idx = self.active_overlay?;
        let (px, py) = self.pointer_local(idx);
        if let Some(layout) = self.toolbar_layout_for(idx) {
            if let Some(a) = layout.hit(px, py) {
                return Some((layout, a));
            }
        }
        if let Some(layout) = self.side_toolbar_layout_for(idx) {
            if let Some(a) = layout.hit(px, py) {
                return Some((layout, a));
            }
        }
        None
    }

    fn pointer_on_any_toolbar_bg(&self) -> bool {
        let Some(idx) = self.active_overlay else {
            return false;
        };
        let (px, py) = self.pointer_local(idx);
        if let Some(layout) = self.toolbar_layout_for(idx) {
            if rect_contains(layout.bg, px, py, 0) {
                return true;
            }
        }
        if let Some(layout) = self.side_toolbar_layout_for(idx) {
            if rect_contains(layout.bg, px, py, 0) {
                return true;
            }
        }
        false
    }

    fn apply_transform_drag(&mut self) {
        let Some(id) = self.canvas.selected() else {
            return;
        };
        let pp = if self.snap_on {
            snap_point_step(self.pointer_pos_global, self.snap_step)
        } else {
            self.pointer_pos_global
        };
        let Some(drag) = self.transform_drag.as_ref() else {
            return;
        };
        let mut new_shape = match drag {
            TransformDrag::Scale {
                anchor,
                start_dist,
                original,
            } => {
                let dx = pp.x - anchor.x;
                let dy = pp.y - anchor.y;
                let d = (dx * dx + dy * dy).sqrt().max(1.0);
                let factor = (d / start_dist).clamp(0.05, 20.0);
                let mut s = (**original).clone();
                crate::canvas::scale_shape_about(&mut s, *anchor, factor);
                s
            }
            TransformDrag::Rotate {
                center,
                start_angle,
                original,
            } => {
                let a = (pp.y - center.y).atan2(pp.x - center.x);
                let mut s = (**original).clone();
                crate::canvas::rotate_shape_about(&mut s, *center, a - start_angle);
                s
            }
        };
        new_shape.id = id;
        self.canvas.replace_shape(id, new_shape);
    }

    fn update_hover_popups(&mut self) {
        // Suppress hover-opens when a HUD is on so moving toward it doesn't
        // flicker popups along the way. Pinned popups are untouched.
        if self.magnifier_on || self.pipette_pending || self.snap_on {
            return;
        }
        let any_pinned = self.picker.as_ref().is_some_and(|p| p.pinned)
            || self.width_popup.as_ref().is_some_and(|p| p.pinned)
            || self.snap_popup.as_ref().is_some_and(|p| p.pinned);

        let Some(idx) = self.active_overlay else {
            return;
        };
        let Some(layout) = self.toolbar_layout_for(idx) else {
            return;
        };
        let (px, py) = self.pointer_local(idx);
        let action = layout.hit(px, py);

        let color_chip = layout.button_rect_for(ButtonAction::TogglePicker);
        let width_chip = layout.button_rect_for(ButtonAction::ToggleWidthPopup);
        let snap_chip = layout.button_rect_for(ButtonAction::ToggleSnapPopup);

        let on_color_chip = matches!(action, Some(ButtonAction::TogglePicker));
        let on_width_chip = matches!(action, Some(ButtonAction::ToggleWidthPopup));
        let on_snap_chip = matches!(action, Some(ButtonAction::ToggleSnapPopup));

        if on_color_chip && self.picker.is_none() && !any_pinned {
            self.color_hover_at = Some(0);
            self.width_popup = None;
            self.snap_popup = None;
            let (x, y) =
                popup_origin_below_toolbar(self, idx, HSV_PICKER_W, HSV_PICKER_H).unwrap_or((0, 0));
            let c = self.current_color;
            let seed = rgb_to_hsv(c.0[0], c.0[1], c.0[2]);
            self.picker = Some(HsvPicker {
                hue: seed.0,
                sat: seed.1,
                val: seed.2,
                origin: (x, y),
                overlay_idx: idx,
                pinned: false,
            });
        } else if let Some(p) = self.picker.as_ref() {
            if !p.pinned {
                let picker_rect = p.outer_rect();
                let in_corridor = match color_chip {
                    Some(chip) => in_chip_popup_corridor(chip, picker_rect, px, py, 6),
                    None => rect_contains(picker_rect, px, py, 6),
                };
                if !in_corridor {
                    self.picker = None;
                    self.color_hover_at = None;
                }
            }
        }

        if on_width_chip && self.width_popup.is_none() && !any_pinned {
            self.width_hover = true;
            self.picker = None;
            self.color_hover_at = None;
            self.snap_popup = None;
            let (x, y) = popup_origin_below_toolbar(self, idx, WIDTH_POPUP_W, WIDTH_POPUP_H)
                .unwrap_or((0, 0));
            self.width_popup = Some(WidthPopup {
                origin: (x, y),
                overlay_idx: idx,
                dragging: false,
                pinned: false,
            });
        } else if let Some(p) = self.width_popup.as_ref() {
            if !p.pinned && !p.dragging {
                let popup_rect = p.outer_rect();
                let in_corridor = match width_chip {
                    Some(c) => in_chip_popup_corridor(c, popup_rect, px, py, 6),
                    None => rect_contains(popup_rect, px, py, 6),
                };
                if !in_corridor {
                    self.width_popup = None;
                    self.width_hover = false;
                }
            }
        } else if !on_width_chip {
            self.width_hover = false;
        }

        if on_snap_chip && self.snap_popup.is_none() && !any_pinned {
            self.picker = None;
            self.color_hover_at = None;
            self.width_popup = None;
            let (x, y) = popup_origin_below_toolbar(self, idx, SNAP_POPUP_W, SNAP_POPUP_H_INNER)
                .unwrap_or((0, 0));
            self.snap_popup = Some(SnapPopup {
                origin: (x, y),
                overlay_idx: idx,
                dragging: false,
                pinned: false,
            });
        } else if let Some(p) = self.snap_popup.as_ref() {
            if !p.pinned && !p.dragging {
                let popup_rect = p.outer_rect();
                let in_corridor = match snap_chip {
                    Some(c) => in_chip_popup_corridor(c, popup_rect, px, py, 6),
                    None => rect_contains(popup_rect, px, py, 6),
                };
                if !in_corridor {
                    self.snap_popup = None;
                }
            }
        }
    }

    /// Click toggles closed → open+pin → close. Closes other popups.
    fn open_width_popup(&mut self) {
        if let Some(p) = self.width_popup.as_mut() {
            if p.pinned {
                self.width_popup = None;
            } else {
                p.pinned = true;
            }
            return;
        }
        self.picker = None;
        self.picker_drag = None;
        self.snap_popup = None;
        let Some(idx) = self.active_overlay else {
            return;
        };
        let mon = self.overlays[idx].monitor.bounds();
        let (ox, oy) = popup_origin_below_toolbar(self, idx, WIDTH_POPUP_W, WIDTH_POPUP_H)
            .unwrap_or_else(|| {
                (
                    ((mon.width() as i32 - WIDTH_POPUP_W) / 2).max(8),
                    (mon.height() as i32 - WIDTH_POPUP_H - 24).max(8),
                )
            });
        self.width_popup = Some(WidthPopup {
            origin: (ox, oy),
            overlay_idx: idx,
            dragging: false,
            pinned: true,
        });
    }

    fn open_snap_popup(&mut self) {
        if let Some(p) = self.snap_popup.as_mut() {
            if p.pinned {
                self.snap_popup = None;
            } else {
                p.pinned = true;
            }
            return;
        }
        self.picker = None;
        self.picker_drag = None;
        self.width_popup = None;
        let Some(idx) = self.active_overlay else {
            return;
        };
        let mon = self.overlays[idx].monitor.bounds();
        let (ox, oy) = popup_origin_below_toolbar(self, idx, SNAP_POPUP_W, SNAP_POPUP_H_INNER)
            .unwrap_or_else(|| {
                (
                    ((mon.width() as i32 - SNAP_POPUP_W) / 2).max(8),
                    (mon.height() as i32 - SNAP_POPUP_H_INNER - 24).max(8),
                )
            });
        self.snap_popup = Some(SnapPopup {
            origin: (ox, oy),
            overlay_idx: idx,
            dragging: false,
            pinned: true,
        });
    }

    /// Reseat the active tool's brush settings from the persistent values.
    fn push_current_to_tool(&mut self) {
        set_active_tool_color(&mut self.canvas.active_tool, self.current_color);
        set_active_tool_width(&mut self.canvas.active_tool, self.current_width);
        self.canvas.set_fill_color(self.current_fill);
    }

    /// Apply a button click; returns true so the caller can swallow the click.
    fn apply_button(&mut self, action: ButtonAction) -> bool {
        match action {
            ButtonAction::SelectTool(i) => {
                if let Some(t) = self.config.palette.tools.get(i).cloned() {
                    self.canvas.set_tool(t);
                    // Clearing only the canvas fill isn't enough — the next
                    // push_current_to_tool would re-apply current_fill.
                    self.current_fill = None;
                    self.push_current_to_tool();
                    mark_all_redraw(self);
                }
            }
            ButtonAction::SelectToolFilled(i) => {
                if let Some(t) = self.config.palette.tools.get(i).cloned() {
                    self.canvas.set_tool(t);
                    self.current_fill = Some(self.current_color);
                    self.canvas.set_fill_color(Some(self.current_color));
                    self.push_current_to_tool();
                    mark_all_redraw(self);
                }
            }
            ButtonAction::ClearAll => {
                self.canvas.clear_shapes();
                mark_all_redraw(self);
            }
            ButtonAction::TogglePicker => {
                // Click toggles closed → open+pin → close.
                if let Some(p) = self.picker.as_mut() {
                    if p.pinned {
                        self.picker = None;
                        self.picker_drag = None;
                    } else {
                        p.pinned = true;
                    }
                } else if let Some(idx) = self.active_overlay {
                    self.width_popup = None;
                    let (x, y) = popup_origin_below_toolbar(self, idx, HSV_PICKER_W, HSV_PICKER_H)
                        .unwrap_or_else(|| {
                            let mon = self.overlays[idx].monitor.bounds();
                            (
                                ((mon.width() as i32 - HSV_PICKER_W) / 2).max(8),
                                (mon.height() as i32 - HSV_PICKER_H - 24).max(8),
                            )
                        });
                    let c = self.current_color;
                    let seed = rgb_to_hsv(c.0[0], c.0[1], c.0[2]);
                    self.picker = Some(HsvPicker {
                        hue: seed.0,
                        sat: seed.1,
                        val: seed.2,
                        origin: (x, y),
                        overlay_idx: idx,
                        pinned: true,
                    });
                }
                mark_all_redraw(self);
            }
            ButtonAction::ToggleWidthPopup => {
                self.open_width_popup();
                mark_all_redraw(self);
            }
            ButtonAction::ToggleSnapPopup => {
                self.open_snap_popup();
                mark_all_redraw(self);
            }
            ButtonAction::RaiseSelected => {
                self.canvas.raise_selected();
                mark_all_redraw(self);
            }
            ButtonAction::LowerSelected => {
                self.canvas.lower_selected();
                mark_all_redraw(self);
            }
            ButtonAction::ToggleSnap => {
                self.snap_on = !self.snap_on;
                mark_all_redraw(self);
            }
            ButtonAction::ToggleMagnifier => {
                self.magnifier_on = !self.magnifier_on;
                mark_all_redraw(self);
            }
            ButtonAction::TogglePipette => {
                self.pipette_pending = !self.pipette_pending;
                mark_all_redraw(self);
            }
            ButtonAction::DeleteSelected => {
                self.canvas.handle(CanvasEvent::Delete);
                mark_all_redraw(self);
            }
            ButtonAction::Undo => {
                self.canvas.handle(CanvasEvent::Undo);
                mark_all_redraw(self);
            }
            ButtonAction::Redo => {
                self.canvas.handle(CanvasEvent::Redo);
                mark_all_redraw(self);
            }
            ButtonAction::Cancel => {
                self.outcome = Some(Outcome::Cancelled);
                self.running = false;
            }
            ButtonAction::Confirm => confirm(self),
            ButtonAction::Copy => {
                self.action.copy = true;
                confirm(self);
            }
            ButtonAction::Save => {
                self.action.save = true;
                confirm(self);
            }
        }
        true
    }
}

/// Popup origin just below the toolbar, or `None` when the toolbar is hidden.
fn popup_origin_below_toolbar(
    state: &State,
    idx: usize,
    popup_w: i32,
    popup_h: i32,
) -> Option<(i32, i32)> {
    let layout = state.toolbar_layout_for(idx)?;
    let (tb_x, tb_y, tb_w, tb_h) = layout.bg;
    let mon = state.overlays[idx].monitor.bounds();
    let mon_w = mon.width() as i32;
    let mon_h = mon.height() as i32;

    let cx = tb_x + tb_w as i32 / 2;
    let mut x = cx - popup_w / 2;
    x = x.clamp(8, mon_w - popup_w - 8);
    let mut y = tb_y + tb_h as i32 + 8;
    if y + popup_h > mon_h - 8 {
        y = tb_y - popup_h - 8;
    }
    y = y.clamp(8, mon_h - popup_h - 8);
    Some((x, y))
}

fn tool_icon(t: &crate::tool::Tool) -> ToolbarIcon {
    use crate::tool::Tool;
    match t {
        Tool::Pointer => ToolbarIcon::Pointer,
        Tool::Brush(_) => ToolbarIcon::Brush,
        Tool::Line(_) => ToolbarIcon::Line,
        Tool::Arrow(_) => ToolbarIcon::Arrow,
        Tool::Rectangle(_) => ToolbarIcon::Rectangle,
        Tool::Ellipse(_) => ToolbarIcon::Ellipse,
        Tool::BlurRect { .. } => ToolbarIcon::Blur,
        Tool::Eraser { .. } => ToolbarIcon::Eraser,
        Tool::Step(_) => ToolbarIcon::Step,
        Tool::Text(_) => ToolbarIcon::Text,
        Tool::Polygon(_) => ToolbarIcon::Polygon,
    }
}

fn filled_tool_icon(t: &crate::tool::Tool) -> ToolbarIcon {
    use crate::tool::Tool;
    match t {
        Tool::Rectangle(_) => ToolbarIcon::RectangleFilled,
        Tool::Ellipse(_) => ToolbarIcon::EllipseFilled,
        Tool::Polygon(_) => ToolbarIcon::PolygonFilled,
        _ => tool_icon(t),
    }
}

fn set_active_tool_width(t: &mut crate::tool::Tool, width: f32) {
    use crate::tool::Tool;
    match t {
        Tool::Brush(b)
        | Tool::Line(b)
        | Tool::Arrow(b)
        | Tool::Rectangle(b)
        | Tool::Ellipse(b)
        | Tool::Polygon(b) => b.width = width,
        Tool::Step(s) => s.radius = (width * 4.0 + 4.0).max(6.0),
        _ => {}
    }
}

fn set_active_tool_color(t: &mut crate::tool::Tool, color: crate::color::Color) {
    use crate::tool::Tool;
    match t {
        Tool::Brush(b)
        | Tool::Line(b)
        | Tool::Arrow(b)
        | Tool::Rectangle(b)
        | Tool::Ellipse(b)
        | Tool::Polygon(b) => b.color = color,
        Tool::Step(s) => s.fill = color,
        Tool::Text(t) => t.color = color,
        _ => {}
    }
}

const DELETE_BTN_W: u32 = 24;
const DELETE_BTN_H: u32 = 24;
const DELETE_BTN_OFFSET: i32 = 4;
const SEL_BTN_GAP: i32 = 4;

#[derive(Clone, Copy, Debug)]
pub(crate) struct SelectionButton {
    pub rect: Shape,
    pub action: ButtonAction,
    pub icon: ToolbarIcon,
    pub tint: [u8; 3],
}

fn selection_decor_layout(
    overlay_idx: usize,
    state: &State,
) -> Option<(
    /*shape*/ Shape,
    Vec<SelectionButton>,
    /*scale handle*/ Shape,
    /*rotate handle*/ Shape,
)> {
    if !matches!(state.canvas.active_tool, crate::tool::Tool::Pointer) {
        return None;
    }
    let sel_id = state.canvas.selected()?;
    let shape = state.canvas.shapes().iter().find(|s| s.id == sel_id)?;
    let bounds = shape.bounds();
    let mon = state.overlays[overlay_idx].monitor.bounds();

    // Only render decor on the overlay with the largest intersection so the
    // gizmo doesn't appear on every monitor at the matching local position.
    let inter = match mon.intersection(&bounds) {
        Some(r) if r.width() > 0 && r.height() > 0 => r,
        _ => return None,
    };
    let my_area = inter.width() as u64 * inter.height() as u64;
    for (i, ov) in state.overlays.iter().enumerate() {
        if i == overlay_idx {
            continue;
        }
        if let Some(r) = ov.monitor.bounds().intersection(&bounds) {
            let a = r.width() as u64 * r.height() as u64;
            if a > my_area {
                return None;
            }
        }
    }

    let local_x = bounds.x() - mon.x();
    let local_y = bounds.y() - mon.y();
    let shape_rect = (local_x, local_y, bounds.width(), bounds.height());

    let entries: [(ButtonAction, ToolbarIcon, [u8; 3]); 3] = [
        (
            ButtonAction::RaiseSelected,
            ToolbarIcon::Raise,
            [60, 60, 64],
        ),
        (
            ButtonAction::LowerSelected,
            ToolbarIcon::Lower,
            [60, 60, 64],
        ),
        (
            ButtonAction::DeleteSelected,
            ToolbarIcon::Trash,
            [180, 40, 40],
        ),
    ];
    let n = entries.len() as i32;
    let total_w = n * DELETE_BTN_W as i32 + (n - 1) * SEL_BTN_GAP;
    let mon_w = mon.width() as i32;
    let mon_h = mon.height() as i32;

    let mut bar_x = local_x + bounds.width() as i32 - total_w + DELETE_BTN_OFFSET;
    bar_x = bar_x.clamp(4, (mon_w - total_w - 4).max(4));

    // Reserve room above the action bar for the rotate handle.
    let extra_for_rotate = DELETE_BTN_H as i32 + 8;
    let above = local_y - DELETE_BTN_H as i32 - DELETE_BTN_OFFSET - extra_for_rotate;
    let below = local_y + bounds.height() as i32 + DELETE_BTN_OFFSET;
    let mut bar_y = if above >= 4 {
        above
    } else if below + DELETE_BTN_H as i32 <= mon_h - 4 {
        below
    } else {
        local_y.max(4)
    };
    bar_y = bar_y.clamp(4, (mon_h - DELETE_BTN_H as i32 - 4).max(4));

    let mut buttons = Vec::with_capacity(entries.len());
    for (i, (action, icon, tint)) in entries.iter().enumerate() {
        let x = bar_x + i as i32 * (DELETE_BTN_W as i32 + SEL_BTN_GAP);
        buttons.push(SelectionButton {
            rect: (x, bar_y, DELETE_BTN_W, DELETE_BTN_H),
            action: *action,
            icon: *icon,
            tint: *tint,
        });
    }

    let scale_x = (local_x + bounds.width() as i32 - DELETE_BTN_W as i32 / 2)
        .clamp(4, (mon_w - DELETE_BTN_W as i32 - 4).max(4));
    let scale_y = (local_y + bounds.height() as i32 - DELETE_BTN_H as i32 / 2)
        .clamp(4, (mon_h - DELETE_BTN_H as i32 - 4).max(4));
    let scale_rect = (scale_x, scale_y, DELETE_BTN_W, DELETE_BTN_H);

    let rotate_x = (local_x + bounds.width() as i32 / 2 - DELETE_BTN_W as i32 / 2)
        .clamp(4, (mon_w - DELETE_BTN_W as i32 - 4).max(4));
    let rotate_y = (local_y - DELETE_BTN_H as i32 - DELETE_BTN_OFFSET).max(4);
    let rotate_rect = (rotate_x, rotate_y, DELETE_BTN_W, DELETE_BTN_H);

    Some((shape_rect, buttons, scale_rect, rotate_rect))
}

/// Paint the dashed outline on every overlay the shape's bounds touches.
fn draw_selection_outline_if_visible(bytes: &mut [u8], w: u32, h: u32, idx: usize, state: &State) {
    if !matches!(state.canvas.active_tool, crate::tool::Tool::Pointer) {
        return;
    }
    let Some(sel_id) = state.canvas.selected() else {
        return;
    };
    let Some(shape) = state.canvas.shapes().iter().find(|s| s.id == sel_id) else {
        return;
    };
    let bounds = shape.bounds();
    let mon = state.overlays[idx].monitor.bounds();
    if mon.intersection(&bounds).is_none() {
        return;
    }
    let local_x = bounds.x() - mon.x();
    let local_y = bounds.y() - mon.y();
    draw_dashed_rect(
        bytes,
        w,
        h,
        (local_x, local_y, bounds.width(), bounds.height()),
        [255, 255, 255],
    );
}

fn draw_selection_decor(bytes: &mut [u8], w: u32, h: u32, idx: usize, state: &State) {
    draw_selection_outline_if_visible(bytes, w, h, idx, state);

    let (shape_rect, buttons, scale_rect, rotate_rect) = match selection_decor_layout(idx, state) {
        Some(r) => r,
        None => return,
    };

    for b in &buttons {
        fill_rect_bytes(bytes, w, h, b.rect, b.tint);
        outline_rect(bytes, w, h, b.rect, [255, 220, 220]);
        let cx = b.rect.0 + b.rect.2 as i32 / 2;
        let cy = b.rect.1 + b.rect.3 as i32 / 2;
        draw_icon(bytes, w, h, cx, cy, b.icon, [240, 240, 240]);
    }

    fill_rect_bytes(bytes, w, h, scale_rect, [60, 110, 200]);
    outline_rect(bytes, w, h, scale_rect, [200, 220, 255]);
    let cx = scale_rect.0 + scale_rect.2 as i32 / 2;
    let cy = scale_rect.1 + scale_rect.3 as i32 / 2;
    draw_icon(
        bytes,
        w,
        h,
        cx,
        cy,
        ToolbarIcon::GizmoScale,
        [240, 240, 240],
    );
    let stem_x = rotate_rect.0 + rotate_rect.2 as i32 / 2;
    let stem_y0 = shape_rect.1;
    let stem_y1 = rotate_rect.1 + rotate_rect.3 as i32;
    for y in stem_y1..stem_y0 {
        paint_pixel(bytes, w, h, stem_x as u32, y as u32, [200, 220, 255]);
    }
    fill_rect_bytes(bytes, w, h, rotate_rect, [200, 130, 60]);
    outline_rect(bytes, w, h, rotate_rect, [255, 220, 200]);
    let cx = rotate_rect.0 + rotate_rect.2 as i32 / 2;
    let cy = rotate_rect.1 + rotate_rect.3 as i32 / 2;
    draw_icon(
        bytes,
        w,
        h,
        cx,
        cy,
        ToolbarIcon::GizmoRotate,
        [240, 240, 240],
    );
}

fn draw_dashed_rect(bytes: &mut [u8], w: u32, h: u32, rect: Shape, rgb: [u8; 3]) {
    let (rx, ry, rw, rh) = rect;
    if rw == 0 || rh == 0 {
        return;
    }
    let on = 6;
    let off = 4;
    let stride = on + off;
    for x in 0..rw as i32 {
        if (x % stride) < on {
            paint_pixel(bytes, w, h, (rx + x) as u32, ry as u32, rgb);
            paint_pixel(
                bytes,
                w,
                h,
                (rx + x) as u32,
                (ry + rh as i32 - 1) as u32,
                rgb,
            );
        }
    }
    for y in 0..rh as i32 {
        if (y % stride) < on {
            paint_pixel(bytes, w, h, rx as u32, (ry + y) as u32, rgb);
            paint_pixel(
                bytes,
                w,
                h,
                (rx + rw as i32 - 1) as u32,
                (ry + y) as u32,
                rgb,
            );
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GizmoHandle {
    Scale,
    Rotate,
}

fn pointer_on_gizmo_handle(state: &State) -> Option<(GizmoHandle, Shape)> {
    let idx = state.active_overlay?;
    let (shape_rect, _btns, scale_rect, rotate_rect) = selection_decor_layout(idx, state)?;
    let mon = state.overlays[idx].monitor.bounds();
    let px = state.pointer_pos_global.x as i32 - mon.x();
    let py = state.pointer_pos_global.y as i32 - mon.y();
    if rect_contains(scale_rect, px, py, 0) {
        return Some((GizmoHandle::Scale, shape_rect));
    }
    if rect_contains(rotate_rect, px, py, 0) {
        return Some((GizmoHandle::Rotate, shape_rect));
    }
    None
}

fn pointer_on_selection_button(state: &State) -> Option<ButtonAction> {
    let idx = state.active_overlay?;
    let (_shape_rect, buttons, _scale_rect, _rotate_rect) = selection_decor_layout(idx, state)?;
    let mon = state.overlays[idx].monitor.bounds();
    let px = state.pointer_pos_global.x as i32 - mon.x();
    let py = state.pointer_pos_global.y as i32 - mon.y();
    for b in &buttons {
        let (rx, ry, rw, rh) = b.rect;
        if px >= rx && px < rx + rw as i32 && py >= ry && py < ry + rh as i32 {
            return Some(b.action);
        }
    }
    None
}

fn draw_line_simple(bytes: &mut [u8], w: u32, h: u32, a: (i32, i32), b: (i32, i32), rgb: [u8; 3]) {
    let (mut x0, mut y0) = a;
    let (x1, y1) = b;
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        paint_pixel(bytes, w, h, x0 as u32, y0 as u32, rgb);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

const HSV_PICKER_W: i32 = 260;
const HSV_HUE_H: i32 = 18;
const HSV_GAP_Y: i32 = 6;
const HSV_SV_H: i32 = 140;
const HSV_FOOT_H: i32 = 22;
const HSV_PICKER_H: i32 = HSV_HUE_H + HSV_GAP_Y + HSV_SV_H + HSV_GAP_Y + HSV_FOOT_H;
#[allow(dead_code)]
const HSV_PICKER_GAP: i32 = 8;

pub(crate) struct HsvPicker {
    pub hue: f32,
    pub sat: f32,
    pub val: f32,
    pub origin: (i32, i32),
    pub overlay_idx: usize,
    /// `true` when clicked → stays open; `false` when opened by hover.
    pub pinned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HsvHit {
    Hue,
    Sv,
}

impl HsvPicker {
    fn outer_rect(&self) -> Shape {
        (
            self.origin.0,
            self.origin.1,
            HSV_PICKER_W as u32,
            HSV_PICKER_H as u32,
        )
    }
    fn hue_rect(&self) -> Shape {
        (
            self.origin.0,
            self.origin.1,
            HSV_PICKER_W as u32,
            HSV_HUE_H as u32,
        )
    }
    fn sv_rect(&self) -> Shape {
        (
            self.origin.0,
            self.origin.1 + HSV_HUE_H + HSV_GAP_Y,
            HSV_PICKER_W as u32,
            HSV_SV_H as u32,
        )
    }
    fn foot_rect(&self) -> Shape {
        (
            self.origin.0,
            self.origin.1 + HSV_HUE_H + HSV_GAP_Y + HSV_SV_H + HSV_GAP_Y,
            HSV_PICKER_W as u32,
            HSV_FOOT_H as u32,
        )
    }

    fn contains(&self, lx: i32, ly: i32) -> bool {
        let (x, y, w, h) = self.outer_rect();
        lx >= x - 4 && lx < x + w as i32 + 4 && ly >= y - 4 && ly < y + h as i32 + 4
    }

    fn hit(&self, lx: i32, ly: i32) -> Option<HsvHit> {
        let inside = |r: Shape, lx: i32, ly: i32| {
            lx >= r.0 && lx < r.0 + r.2 as i32 && ly >= r.1 && ly < r.1 + r.3 as i32
        };
        if inside(self.hue_rect(), lx, ly) {
            return Some(HsvHit::Hue);
        }
        if inside(self.sv_rect(), lx, ly) {
            return Some(HsvHit::Sv);
        }
        None
    }

    fn hue_at(&self, lx: i32) -> f32 {
        let r = self.hue_rect();
        ((lx - r.0).clamp(0, r.2 as i32 - 1) as f32) / (r.2 as f32)
    }

    /// (saturation, value): x is value, y is saturation (Photoshop layout).
    fn sv_at(&self, lx: i32, ly: i32) -> (f32, f32) {
        let r = self.sv_rect();
        let v = ((lx - r.0).clamp(0, r.2 as i32 - 1) as f32) / (r.2 as f32);
        let s = ((ly - r.1).clamp(0, r.3 as i32 - 1) as f32) / (r.3 as f32);
        (s, v)
    }

    pub fn current_rgb(&self) -> [u8; 3] {
        hsv_to_rgb(self.hue, self.sat, self.val)
    }
}

fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let v = max;
    let s = if max <= 0.0 { 0.0 } else { d / max };
    let h = if d <= 0.0 {
        0.0
    } else if (max - r).abs() < f32::EPSILON {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if (max - g).abs() < f32::EPSILON {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, v)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    ]
}

fn draw_hsv_picker(bytes: &mut [u8], w: u32, h: u32, picker: &HsvPicker) {
    let (ox, oy, pw, ph) = picker.outer_rect();
    fill_rect_bytes(
        bytes,
        w,
        h,
        (ox - 6, oy - 6, pw + 12, ph + 12),
        [16, 16, 18],
    );
    outline_rect(
        bytes,
        w,
        h,
        (ox - 6, oy - 6, pw + 12, ph + 12),
        [60, 60, 64],
    );
    fill_rect_bytes(bytes, w, h, (ox - 2, oy - 2, pw + 4, ph + 4), [22, 22, 24]);

    let (hx, hy, hw, hh) = picker.hue_rect();
    for ix in 0..hw as i32 {
        let hue = ix as f32 / hw as f32;
        let rgb = hsv_to_rgb(hue, 1.0, 1.0);
        for iy in 0..hh as i32 {
            paint_pixel(bytes, w, h, (hx + ix) as u32, (hy + iy) as u32, rgb);
        }
    }
    let tick_x = hx + (picker.hue * hw as f32).round() as i32;
    for iy in -2..=hh as i32 + 1 {
        paint_pixel(
            bytes,
            w,
            h,
            tick_x as u32,
            (hy + iy) as u32,
            [255, 255, 255],
        );
        paint_pixel(
            bytes,
            w,
            h,
            (tick_x - 1) as u32,
            (hy + iy) as u32,
            [0, 0, 0],
        );
        paint_pixel(
            bytes,
            w,
            h,
            (tick_x + 1) as u32,
            (hy + iy) as u32,
            [0, 0, 0],
        );
    }

    let (sx, sy, sw, sh) = picker.sv_rect();
    for iy in 0..sh as i32 {
        let s = iy as f32 / sh as f32;
        for ix in 0..sw as i32 {
            let v = ix as f32 / sw as f32;
            let rgb = hsv_to_rgb(picker.hue, s, v);
            paint_pixel(bytes, w, h, (sx + ix) as u32, (sy + iy) as u32, rgb);
        }
    }
    let mx = sx + (picker.val * sw as f32).round() as i32;
    let my = sy + (picker.sat * sh as f32).round() as i32;
    let marker = if picker.val > 0.55 && picker.sat < 0.6 {
        [0, 0, 0]
    } else {
        [255, 255, 255]
    };
    for theta in 0..32 {
        let a = (theta as f32 / 32.0) * std::f32::consts::TAU;
        let r = 6.0;
        let dx = (r * a.cos()).round() as i32;
        let dy = (r * a.sin()).round() as i32;
        paint_pixel(bytes, w, h, (mx + dx) as u32, (my + dy) as u32, marker);
    }

    let (fx, fy, fw, fh) = picker.foot_rect();
    let rgb = picker.current_rgb();
    let swatch_w = 40;
    fill_rect_bytes(bytes, w, h, (fx, fy, swatch_w as u32, fh), rgb);
    outline_rect(bytes, w, h, (fx, fy, swatch_w as u32, fh), [200, 200, 200]);
    let hex = format!(
        "#{:02X}{:02X}{:02X}    H {:>3}  S {:>3}  V {:>3}",
        rgb[0],
        rgb[1],
        rgb[2],
        (picker.hue * 360.0) as i32,
        (picker.sat * 100.0) as i32,
        (picker.val * 100.0) as i32,
    );
    draw_text(
        bytes,
        w,
        h,
        (fx + swatch_w + 8) as u32,
        (fy + 4) as u32,
        &hex,
        [230, 230, 230],
    );
    let _ = fw;
}

const WIDTH_POPUP_W: i32 = 220;
const WIDTH_POPUP_H: i32 = 44;
const WIDTH_MIN: f32 = 1.0;
const WIDTH_MAX: f32 = 50.0;

/// Right-click quick menu: grid of colour swatches plus a row of widths.
pub(crate) struct RadialMenu {
    pub origin: (i32, i32),
    pub overlay_idx: usize,
}

const RADIAL_CELL: i32 = 26;
const RADIAL_GAP: i32 = 4;
const RADIAL_COLS: i32 = 4;
const RADIAL_PAD: i32 = 8;

impl RadialMenu {
    fn outer_rect(&self, palette_len: usize) -> Shape {
        let cols = RADIAL_COLS;
        let n = palette_len.max(1) as i32;
        let rows = (n + cols - 1) / cols;
        let grid_w = cols * RADIAL_CELL + (cols - 1) * RADIAL_GAP;
        let grid_h = rows * RADIAL_CELL + (rows - 1) * RADIAL_GAP;
        let width_row_h = RADIAL_CELL;
        let total_w = grid_w + RADIAL_PAD * 2;
        let total_h = grid_h + RADIAL_GAP + width_row_h + RADIAL_PAD * 2;
        (self.origin.0, self.origin.1, total_w as u32, total_h as u32)
    }

    fn color_cell_rect(&self, palette_len: usize, i: usize) -> Shape {
        let (ox, oy, _, _) = self.outer_rect(palette_len);
        let col = i as i32 % RADIAL_COLS;
        let row = i as i32 / RADIAL_COLS;
        (
            ox + RADIAL_PAD + col * (RADIAL_CELL + RADIAL_GAP),
            oy + RADIAL_PAD + row * (RADIAL_CELL + RADIAL_GAP),
            RADIAL_CELL as u32,
            RADIAL_CELL as u32,
        )
    }

    fn width_row_y(&self, palette_len: usize) -> i32 {
        let (_, oy, _, _) = self.outer_rect(palette_len);
        let n = palette_len.max(1) as i32;
        let rows = (n + RADIAL_COLS - 1) / RADIAL_COLS;
        let grid_h = rows * RADIAL_CELL + (rows - 1) * RADIAL_GAP;
        oy + RADIAL_PAD + grid_h + RADIAL_GAP
    }

    fn width_cell_rect(&self, palette_len: usize, i: usize) -> Shape {
        let (ox, _, _, _) = self.outer_rect(palette_len);
        let y = self.width_row_y(palette_len);
        (
            ox + RADIAL_PAD + i as i32 * (RADIAL_CELL + RADIAL_GAP),
            y,
            RADIAL_CELL as u32,
            RADIAL_CELL as u32,
        )
    }

    fn slot_color(&self, palette_len: usize, lx: i32, ly: i32) -> Option<usize> {
        (0..palette_len).find(|&i| rect_contains(self.color_cell_rect(palette_len, i), lx, ly, 0))
    }

    fn slot_width(&self, lx: i32, ly: i32, palette_len: usize, widths_len: usize) -> Option<usize> {
        (0..widths_len).find(|&i| rect_contains(self.width_cell_rect(palette_len, i), lx, ly, 0))
    }

    fn contains(&self, palette_len: usize, lx: i32, ly: i32) -> bool {
        rect_contains(self.outer_rect(palette_len), lx, ly, 4)
    }
}

/// Active transform-gizmo drag.
#[derive(Clone, Debug)]
pub(crate) enum TransformDrag {
    Scale {
        anchor: FPoint,
        start_dist: f32,
        original: Box<crate::shape::Shape>,
    },
    Rotate {
        center: FPoint,
        start_angle: f32,
        original: Box<crate::shape::Shape>,
    },
}

pub(crate) struct WidthPopup {
    pub origin: (i32, i32),
    pub overlay_idx: usize,
    pub dragging: bool,
    /// `true` when clicked → stays open; `false` when opened by hover.
    pub pinned: bool,
}

impl WidthPopup {
    fn outer_rect(&self) -> Shape {
        (
            self.origin.0,
            self.origin.1,
            WIDTH_POPUP_W as u32,
            WIDTH_POPUP_H as u32,
        )
    }
    fn track_rect(&self) -> Shape {
        (
            self.origin.0 + 12,
            self.origin.1 + 28,
            (WIDTH_POPUP_W - 24) as u32,
            6,
        )
    }
    fn contains(&self, lx: i32, ly: i32) -> bool {
        let (x, y, w, h) = self.outer_rect();
        lx >= x - 6 && lx < x + w as i32 + 6 && ly >= y - 6 && ly < y + h as i32 + 6
    }
    fn value_at(&self, lx: i32) -> f32 {
        let r = self.track_rect();
        let t = ((lx - r.0).clamp(0, r.2 as i32) as f32) / (r.2 as f32);
        (WIDTH_MIN + t * (WIDTH_MAX - WIDTH_MIN))
            .round()
            .clamp(WIDTH_MIN, WIDTH_MAX)
    }
}

fn draw_width_popup(bytes: &mut [u8], w: u32, h: u32, p: &WidthPopup, value: f32) {
    let (ox, oy, pw, ph) = p.outer_rect();
    let panel = [16, 16, 18];
    let border = [60, 60, 64];
    let accent = [90, 170, 255];
    fill_rect_bytes(bytes, w, h, (ox - 6, oy - 6, pw + 12, ph + 12), panel);
    outline_rect(bytes, w, h, (ox - 6, oy - 6, pw + 12, ph + 12), border);
    fill_rect_bytes(bytes, w, h, (ox - 2, oy - 2, pw + 4, ph + 4), [22, 22, 24]);

    let label = format!("Stroke: {}px", value.round() as i32);
    let text_w = super::font::measure(&label, 13.0).ceil() as i32;
    let label_x = ox + (pw as i32 - text_w) / 2;
    draw_text(
        bytes,
        w,
        h,
        label_x.max(ox + 8) as u32,
        (oy + 8) as u32,
        &label,
        [230, 230, 230],
    );

    let (tx, ty, tw, th) = p.track_rect();
    fill_rect_bytes(bytes, w, h, (tx, ty, tw, th), [60, 60, 64]);
    outline_rect(bytes, w, h, (tx, ty, tw, th), [110, 110, 116]);

    let t = ((value - WIDTH_MIN) / (WIDTH_MAX - WIDTH_MIN)).clamp(0.0, 1.0);
    let filled = (t * tw as f32) as u32;
    if filled > 0 {
        fill_rect_bytes(bytes, w, h, (tx, ty, filled, th), accent);
    }

    let thumb_x = tx + filled as i32;
    let thumb_w = 6;
    let thumb_h = 16;
    fill_rect_bytes(
        bytes,
        w,
        h,
        (
            thumb_x - thumb_w / 2,
            ty + th as i32 / 2 - thumb_h / 2,
            thumb_w as u32,
            thumb_h as u32,
        ),
        [230, 230, 230],
    );
}

const SNAP_POPUP_W: i32 = 240;
const SNAP_POPUP_H_INNER: i32 = 110;
const SNAP_STEP_MIN: f32 = 2.0;
const SNAP_STEP_MAX: f32 = 100.0;

pub(crate) struct SnapPopup {
    pub origin: (i32, i32),
    pub overlay_idx: usize,
    pub dragging: bool,
    pub pinned: bool,
}

impl SnapPopup {
    fn outer_rect(&self) -> Shape {
        (
            self.origin.0,
            self.origin.1,
            SNAP_POPUP_W as u32,
            SNAP_POPUP_H_INNER as u32,
        )
    }
    fn track_rect(&self) -> Shape {
        (
            self.origin.0 + 14,
            self.origin.1 + 32,
            (SNAP_POPUP_W - 28) as u32,
            6,
        )
    }
    fn preview_rect(&self) -> Shape {
        (
            self.origin.0 + 14,
            self.origin.1 + 50,
            (SNAP_POPUP_W - 28) as u32,
            (SNAP_POPUP_H_INNER - 50 - 12) as u32,
        )
    }
    fn contains(&self, lx: i32, ly: i32) -> bool {
        let (x, y, w, h) = self.outer_rect();
        lx >= x - 6 && lx < x + w as i32 + 6 && ly >= y - 6 && ly < y + h as i32 + 6
    }
    fn track_hit(&self, lx: i32, ly: i32) -> bool {
        let (rx, ry, rw, rh) = self.track_rect();
        lx >= rx && lx <= rx + rw as i32 && ly >= ry - 6 && ly <= ry + rh as i32 + 6
    }
    fn value_at(&self, lx: i32) -> f32 {
        let r = self.track_rect();
        let t = ((lx - r.0).clamp(0, r.2 as i32) as f32) / (r.2 as f32);
        (SNAP_STEP_MIN + t * (SNAP_STEP_MAX - SNAP_STEP_MIN))
            .round()
            .clamp(SNAP_STEP_MIN, SNAP_STEP_MAX)
    }
}

fn draw_snap_popup(bytes: &mut [u8], w: u32, h: u32, p: &SnapPopup, value: f32) {
    let (ox, oy, pw, ph) = p.outer_rect();
    let panel = [18, 24, 28];
    let border = [70, 90, 100];
    let accent = [80, 200, 200];

    fill_rect_bytes(bytes, w, h, (ox - 6, oy - 6, pw + 12, ph + 12), panel);
    outline_rect(bytes, w, h, (ox - 6, oy - 6, pw + 12, ph + 12), border);
    fill_rect_bytes(bytes, w, h, (ox - 2, oy - 2, pw + 4, ph + 4), [22, 28, 32]);

    let label = format!("Snap step: {}px", value.round() as i32);
    let text_w = super::font::measure(&label, 13.0).ceil() as i32;
    let label_x = ox + (pw as i32 - text_w) / 2;
    draw_text(
        bytes,
        w,
        h,
        label_x.max(ox + 8) as u32,
        (oy + 8) as u32,
        &label,
        [230, 230, 230],
    );

    let (tx, ty, tw, th) = p.track_rect();
    fill_rect_bytes(bytes, w, h, (tx, ty, tw, th), [60, 70, 76]);
    outline_rect(bytes, w, h, (tx, ty, tw, th), [110, 130, 140]);

    let t = ((value - SNAP_STEP_MIN) / (SNAP_STEP_MAX - SNAP_STEP_MIN)).clamp(0.0, 1.0);
    let filled = (t * tw as f32) as u32;
    if filled > 0 {
        fill_rect_bytes(bytes, w, h, (tx, ty, filled, th), accent);
    }
    let thumb_x = tx + filled as i32;
    let thumb_w = 6;
    let thumb_h = 16;
    fill_rect_bytes(
        bytes,
        w,
        h,
        (
            thumb_x - thumb_w / 2,
            ty + th as i32 / 2 - thumb_h / 2,
            thumb_w as u32,
            thumb_h as u32,
        ),
        [230, 230, 230],
    );

    let preview = p.preview_rect();
    fill_rect_bytes(bytes, w, h, preview, [10, 16, 20]);
    outline_rect(bytes, w, h, preview, [60, 90, 100]);
    let step = value.round().max(SNAP_STEP_MIN) as i32;
    let dot = 3i32;
    // Centre the first row vertically so a dot is always visible at large
    // steps (the preview is ~48 px tall, so step >= 50 fits only one row).
    let band_h = preview.3 as i32;
    let first_row = preview.1 + ((band_h - dot) / 2).max(0);
    let first_col = preview.0 + 4;
    let mut gy = first_row;
    while gy + dot <= preview.1 + band_h {
        let mut gx = first_col;
        while gx + dot <= preview.0 + preview.2 as i32 {
            for dy in 0..dot {
                for dx in 0..dot {
                    paint_pixel(bytes, w, h, (gx + dx) as u32, (gy + dy) as u32, accent);
                }
            }
            gx += step;
        }
        gy += step;
    }
}

impl State {
    fn active_tool_width(&self) -> f32 {
        self.current_width
    }

    fn update_current_width(&mut self, w: f32) {
        self.current_width = w.clamp(WIDTH_MIN, WIDTH_MAX);
        set_active_tool_width(&mut self.canvas.active_tool, self.current_width);
    }

    fn step_active_tool_width(&mut self, delta: f32) {
        let cur = self.current_width;
        self.update_current_width(cur + delta);
    }

    fn pointer_in_picker(&self) -> bool {
        let Some(p) = self.picker.as_ref() else {
            return false;
        };
        let Some(idx) = self.active_overlay else {
            return false;
        };
        if idx != p.overlay_idx {
            return false;
        }
        let (lx, ly) = self.pointer_local(idx);
        p.contains(lx, ly)
    }

    fn pointer_in_width_popup(&self) -> bool {
        let Some(p) = self.width_popup.as_ref() else {
            return false;
        };
        let Some(idx) = self.active_overlay else {
            return false;
        };
        if idx != p.overlay_idx {
            return false;
        }
        let (lx, ly) = self.pointer_local(idx);
        p.contains(lx, ly)
    }

    fn pointer_in_snap_popup(&self) -> bool {
        let Some(p) = self.snap_popup.as_ref() else {
            return false;
        };
        let Some(idx) = self.active_overlay else {
            return false;
        };
        if idx != p.overlay_idx {
            return false;
        }
        let (lx, ly) = self.pointer_local(idx);
        p.contains(lx, ly)
    }

    fn pointer_on_snap_track(&self) -> bool {
        let Some(p) = self.snap_popup.as_ref() else {
            return false;
        };
        let Some(idx) = self.active_overlay else {
            return false;
        };
        if idx != p.overlay_idx {
            return false;
        }
        let (lx, ly) = self.pointer_local(idx);
        p.track_hit(lx, ly)
    }

    fn apply_width_slider(&mut self, lx: i32, _ly: i32) {
        let value = {
            let Some(p) = self.width_popup.as_ref() else {
                return;
            };
            p.value_at(lx)
        };
        self.update_current_width(value);
    }

    fn apply_snap_slider(&mut self, lx: i32, _ly: i32) {
        let value = {
            let Some(p) = self.snap_popup.as_ref() else {
                return;
            };
            p.value_at(lx)
        };
        self.snap_step = value.clamp(SNAP_STEP_MIN, SNAP_STEP_MAX);
    }

    fn pointer_on_width_button(&self) -> bool {
        let Some(idx) = self.active_overlay else {
            return false;
        };
        let Some(layout) = self.toolbar_layout_for(idx) else {
            return false;
        };
        let (px, py) = self.pointer_local(idx);
        if let Some(action) = layout.hit(px, py) {
            matches!(action, ButtonAction::ToggleWidthPopup)
        } else {
            false
        }
    }
}

fn draw_radial_menu(
    bytes: &mut [u8],
    w: u32,
    h: u32,
    menu: &RadialMenu,
    palette: &[crate::color::Color],
    widths: &[f32],
    hover_color: Option<usize>,
    hover_width: Option<usize>,
) {
    let palette_len = palette.len();
    let outer = menu.outer_rect(palette_len);
    fill_rect_bytes(bytes, w, h, outer, [22, 22, 24]);
    outline_rect(bytes, w, h, outer, [80, 80, 84]);

    for (i, c) in palette.iter().enumerate() {
        let r = menu.color_cell_rect(palette_len, i);
        fill_rect_bytes(bytes, w, h, r, [c.0[0], c.0[1], c.0[2]]);
        let border = if hover_color == Some(i) {
            [255, 255, 255]
        } else {
            [80, 80, 84]
        };
        outline_rect(bytes, w, h, r, border);
    }

    for (i, w_val) in widths.iter().enumerate() {
        let r = menu.width_cell_rect(palette_len, i);
        let bg = [38, 38, 42];
        fill_rect_bytes(bytes, w, h, r, bg);
        let border = if hover_width == Some(i) {
            [255, 220, 80]
        } else {
            [80, 80, 84]
        };
        outline_rect(bytes, w, h, r, border);
        let cx = r.0 + r.2 as i32 / 2;
        let cy = r.1 + r.3 as i32 / 2;
        let radius = (w_val.round() as i32).clamp(2, (RADIAL_CELL - 8) / 2);
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy > radius * radius {
                    continue;
                }
                paint_pixel(
                    bytes,
                    w,
                    h,
                    (cx + dx) as u32,
                    (cy + dy) as u32,
                    [230, 230, 230],
                );
            }
        }
    }

    let readout = match (hover_color, hover_width) {
        (Some(i), _) if i < palette.len() => {
            let c = palette[i];
            Some(format!("#{:02X}{:02X}{:02X}", c.0[0], c.0[1], c.0[2]))
        }
        (_, Some(j)) if j < widths.len() => Some(format!("W {}px", widths[j].round() as i32)),
        _ => None,
    };
    if let Some(text) = readout {
        let (ox, oy, ow, oh) = outer;
        let y = oy + oh as i32 + 4;
        let rect = (ox, y, ow, 18u32);
        fill_rect_bytes(bytes, w, h, rect, [16, 16, 18]);
        outline_rect(bytes, w, h, rect, [60, 60, 64]);
        draw_text(
            bytes,
            w,
            h,
            (ox + 8) as u32,
            (y + 2) as u32,
            &text,
            [240, 240, 240],
        );
    }
}

/// Paint a faint grid of dots clipped to `clip` (monitor-local), aligned
/// to the global step grid via `mon_origin`.
fn draw_snap_grid_clip(
    bytes: &mut [u8],
    w: u32,
    h: u32,
    step: f32,
    clip: Shape,
    mon_origin: (i32, i32),
) {
    let step = step.max(2.0) as i32;
    let col = [180, 180, 200];
    let (cx, cy, cw, ch) = clip;
    let x0 = cx.max(0);
    let y0 = cy.max(0);
    let x1 = (cx + cw as i32).min(w as i32);
    let y1 = (cy + ch as i32).min(h as i32);
    // First local grid line whose global position is a multiple of step.
    let off_x = (-mon_origin.0).rem_euclid(step);
    let off_y = (-mon_origin.1).rem_euclid(step);
    let start_x = x0 + ((off_x - x0 % step + step) % step);
    let start_y = y0 + ((off_y - y0 % step + step) % step);
    let mut y = start_y;
    while y < y1 {
        let mut x = start_x;
        while x < x1 {
            if x >= x0 && y >= y0 {
                paint_pixel(bytes, w, h, x as u32, y as u32, col);
                paint_pixel(bytes, w, h, (x + 1) as u32, y as u32, col);
                paint_pixel(bytes, w, h, x as u32, (y + 1) as u32, col);
                paint_pixel(bytes, w, h, (x + 1) as u32, (y + 1) as u32, col);
            }
            x += step;
        }
        y += step;
    }
}

const MAGNIFIER_RADIUS: i32 = 64;
const MAGNIFIER_ZOOM: i32 = 4;

fn draw_magnifier(bytes: &mut [u8], w: u32, h: u32, idx: usize, state: &State) {
    // The pipette also opens the magnifier so the sampled pixel is visible.
    if !state.magnifier_on && !state.pipette_pending {
        return;
    }
    let mon = state.overlays[idx].monitor.bounds();
    let cx = state.pointer_pos_global.x as i32 - mon.x();
    let cy = state.pointer_pos_global.y as i32 - mon.y();
    let off_x = MAGNIFIER_RADIUS + 16;
    let off_y = -(MAGNIFIER_RADIUS + 16);
    let ox = cx + off_x;
    let oy = cy + off_y;

    let bg = match state.background.as_ref() {
        Some(b) => b.as_rgba(),
        None => return,
    };
    let monitors_bb = MonitorRect::bounding(
        &state
            .overlays
            .iter()
            .map(|o| o.monitor.bounds())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();
    let bg_origin = (mon.x() - monitors_bb.x(), mon.y() - monitors_bb.y());

    let r2 = MAGNIFIER_RADIUS * MAGNIFIER_RADIUS;
    for dy in -MAGNIFIER_RADIUS..=MAGNIFIER_RADIUS {
        for dx in -MAGNIFIER_RADIUS..=MAGNIFIER_RADIUS {
            let dist2 = dx * dx + dy * dy;
            if dist2 > r2 {
                continue;
            }
            let sx = cx + (dx as f32 / MAGNIFIER_ZOOM as f32).round() as i32;
            let sy = cy + (dy as f32 / MAGNIFIER_ZOOM as f32).round() as i32;
            let bx = sx + bg_origin.0;
            let by = sy + bg_origin.1;
            if bx < 0 || by < 0 || bx as u32 >= bg.width() || by as u32 >= bg.height() {
                continue;
            }
            let pixel = bg.get_pixel(bx as u32, by as u32);
            paint_pixel(
                bytes,
                w,
                h,
                (ox + dx) as u32,
                (oy + dy) as u32,
                [pixel.0[0], pixel.0[1], pixel.0[2]],
            );
        }
    }
    for theta in 0..360 {
        let a = (theta as f32).to_radians();
        let mx = ox + (MAGNIFIER_RADIUS as f32 * a.cos()).round() as i32;
        let my = oy + (MAGNIFIER_RADIUS as f32 * a.sin()).round() as i32;
        paint_pixel(bytes, w, h, mx as u32, my as u32, [255, 255, 255]);
    }
    for d in -2..=2 {
        paint_pixel(bytes, w, h, (ox + d) as u32, oy as u32, [255, 255, 255]);
        paint_pixel(bytes, w, h, ox as u32, (oy + d) as u32, [255, 255, 255]);
    }

    let center_bx = cx + bg_origin.0;
    let center_by = cy + bg_origin.1;
    if center_bx >= 0
        && center_by >= 0
        && (center_bx as u32) < bg.width()
        && (center_by as u32) < bg.height()
    {
        let p = bg.get_pixel(center_bx as u32, center_by as u32);
        let hex = format!("#{:02X}{:02X}{:02X}", p.0[0], p.0[1], p.0[2]);
        let label_y = oy + MAGNIFIER_RADIUS + 8;
        fill_rect_bytes(bytes, w, h, (ox - 40, label_y - 2, 80, 18), [16, 16, 18]);
        outline_rect(bytes, w, h, (ox - 40, label_y - 2, 80, 18), [60, 60, 64]);
        draw_text(
            bytes,
            w,
            h,
            (ox - 32) as u32,
            label_y as u32,
            &hex,
            [240, 240, 240],
        );
    }
}

const SNAP_TOL: f32 = 8.0;

fn snap_point_step(p: FPoint, step: f32) -> FPoint {
    let step = step.max(2.0);
    FPoint::new((p.x / step).round() * step, (p.y / step).round() * step)
}

fn snap_point(canvas: &Canvas, p: FPoint, step: f32) -> FPoint {
    let mut best = snap_point_step(p, step);
    let mut best_d2 = (best.x - p.x).powi(2) + (best.y - p.y).powi(2);
    for shape in canvas.shapes() {
        for v in shape_vertices(shape) {
            let d2 = (v.x - p.x).powi(2) + (v.y - p.y).powi(2);
            if d2 < SNAP_TOL * SNAP_TOL && d2 < best_d2 {
                best = v;
                best_d2 = d2;
            }
        }
    }
    best
}

fn shape_vertices(shape: &crate::shape::Shape) -> Vec<FPoint> {
    use crate::shape::ShapeKind;
    match &shape.kind {
        ShapeKind::FreehandStroke { points } | ShapeKind::Polygon { points, .. } => points.clone(),
        ShapeKind::Line { from, to } | ShapeKind::Arrow { from, to } => vec![*from, *to],
        ShapeKind::Rectangle { rect }
        | ShapeKind::Ellipse { rect }
        | ShapeKind::BlurRect { rect, .. } => {
            let x0 = rect.x() as f32;
            let y0 = rect.y() as f32;
            let x1 = x0 + rect.width() as f32;
            let y1 = y0 + rect.height() as f32;
            vec![
                FPoint::new(x0, y0),
                FPoint::new(x1, y0),
                FPoint::new(x0, y1),
                FPoint::new(x1, y1),
            ]
        }
        ShapeKind::Step { center, .. } => vec![*center],
        ShapeKind::Text { origin, .. } => vec![*origin],
    }
}

impl State {
    fn pipette_apply(&mut self) {
        let bg = match self.background.as_ref() {
            Some(b) => b.as_rgba(),
            None => return,
        };
        let monitors_bb = MonitorRect::bounding(
            &self
                .overlays
                .iter()
                .map(|o| o.monitor.bounds())
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();
        let bx = (self.pointer_pos_global.x as i32 - monitors_bb.x()).max(0) as u32;
        let by = (self.pointer_pos_global.y as i32 - monitors_bb.y()).max(0) as u32;
        if bx >= bg.width() || by >= bg.height() {
            return;
        }
        let px = bg.get_pixel(bx, by);
        let colour = crate::color::Color::rgb(px.0[0], px.0[1], px.0[2]);
        self.apply_pick_color(colour);
    }

    /// Shift+pick targets fill only; without Shift the stroke updates
    /// (and the fill colour tracks it when fill mode is on).
    fn apply_pick_color(&mut self, colour: crate::color::Color) {
        if self.mods.shift {
            self.canvas.set_fill_color(Some(colour));
            self.current_fill = Some(colour);
            return;
        }
        self.current_color = colour;
        set_active_tool_color(&mut self.canvas.active_tool, colour);
        if self.canvas.fill_mode() {
            self.canvas.set_fill_color(Some(colour));
            self.current_fill = Some(colour);
        }
    }

    fn hit_picker(&self) -> Option<(HsvHit, i32, i32)> {
        let p = self.picker.as_ref()?;
        let idx = self.active_overlay?;
        if idx != p.overlay_idx {
            return None;
        }
        let mon = self.overlays[idx].monitor.bounds();
        let lx = self.pointer_pos_global.x as i32 - mon.x();
        let ly = self.pointer_pos_global.y as i32 - mon.y();
        let kind = p.hit(lx, ly)?;
        Some((kind, lx, ly))
    }

    fn apply_picker_hit(&mut self, hit: HsvHit, lx: i32, ly: i32) {
        let rgb = if let Some(p) = self.picker.as_mut() {
            match hit {
                HsvHit::Hue => p.hue = p.hue_at(lx),
                HsvHit::Sv => {
                    let (s, v) = p.sv_at(lx, ly);
                    p.sat = s;
                    p.val = v;
                }
            }
            p.current_rgb()
        } else {
            return;
        };
        let colour = crate::color::Color::rgb(rgb[0], rgb[1], rgb[2]);
        self.apply_pick_color(colour);
    }
}

fn sample_pipette(state: &mut State) {
    state.pipette_apply();
}
