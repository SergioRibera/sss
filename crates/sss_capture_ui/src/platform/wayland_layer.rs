//! Wayland layer-shell driver — bypass winit / wgpu entirely.
//!
//! This is the path that actually works on wlroots compositors (niri, sway,
//! Hyprland, river, cosmic, …). It uses:
//!
//! * `wl_compositor` + `zwlr_layer_shell_v1` to put one overlay surface per
//!   `wl_output`, anchored to all four edges and on the *Overlay* layer.
//!   That guarantees the surface floats above every other client, including
//!   niri's tiling columns — which is what fixes the "windows tiled as a
//!   new column" problem.
//! * `wl_shm` + `memfd_create` + `mmap` for the pixel buffers. No GPU is
//!   involved — we paint with the same CPU compositor that bakes the final
//!   image. This was the *other* immediate problem: four GPU-accelerated
//!   surfaces in `PresentMode::Mailbox` were blendering the desktop in
//!   real-time and taking the kernel down.
//! * `wl_seat` / `wl_pointer` / `wl_keyboard` (with `xkbcommon`) for input,
//!   wired straight into the existing `Canvas` state machine.

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::io::FromRawFd;
use std::time::{Duration, Instant};

use memmap2::MmapMut;
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

/// Cheap probe used by `platform::mod::run` to decide whether to call into
/// this driver. Returns true when the current `WAYLAND_DISPLAY` advertises
/// `zwlr_layer_shell_v1` (every wlroots compositor; KWin in nested-output
/// mode; cosmic). Returns false on GNOME / KDE-Wayland (they don't expose
/// the protocol to regular clients).
/// Dummy state used solely for the layer-shell probe — wayland-client's
/// `registry_queue_init` requires a `Dispatch` impl for the state type
/// even when we're going to drop the queue immediately.
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

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub(crate) fn run(sel: Selector) -> Result<Selection, SelectorError> {
    let Selector { config, capturer } = sel;

    // Always eager-capture: with layer-shell we paint the screenshot under
    // the overlay surface, so we need pixels before we can show anything.
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

    // -------- bind wayland globals -----------------------------------------
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

    // Create the state up front (with empty overlays + empty output infos)
    // so we can bind wl_outputs into it and dispatch their geometry events
    // before we try to match sss_capture monitors to wl_outputs.
    let mut state = State {
        running: true,
        outcome: None,
        action: PostAction {
            copy: false,
            save: false,
            save_path_hint: config.save_path_hint.clone(),
        },
        canvas: Canvas::new(),
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
        snap_step: 10.0,
        current_color: crate::color::Color::RED,
        current_width: 3.0,
        current_fill: None,
        color_hover_at: None,
        width_hover: false,
        radial: None,
        transform_drag: None,
        cursor_ctx: None,
        compositor: Some(compositor.clone()),
        pointer: None,
        qh: Some(qh.clone()),
    };

    // Bind every wl_output advertised at startup, with the registry global
    // name as `Dispatch` user-data so we can populate `output_infos[name]`
    // when geometry / mode events arrive.
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

    // Match each sss_capture::Monitor to its wl_output by *position*. The
    // (x, y) reported by `wl_output.geometry` is in the compositor's
    // coordinate space — same as what sss_capture::Monitor::bounds() uses.
    // Matching by index (the previous behaviour) was wrong: niri's global
    // advertisement order doesn't match sss_capture's monitor ordering, so
    // captures landed on the wrong physical screen.
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

    // -------- input -----------------------------------------------------
    let pointer = seat.get_pointer(&qh, ());
    let keyboard = seat.get_keyboard(&qh, ());
    state.pointer = Some(pointer.clone());
    // Load the system cursor theme so we can swap between
    // `crosshair` / `move` / `nwse-resize` / … based on where the
    // pointer is. Failure here is non-fatal: we just keep the
    // compositor-default cursor.
    match super::cursor::CursorContext::new(&conn, shm.clone(), 24) {
        Ok(ctx) => state.cursor_ctx = Some(ctx),
        Err(e) => tracing::warn!(error = %e, "cursor theme load failed; using compositor default"),
    }

    // -------- main event loop ----------------------------------------------
    while state.running {
        // Pump events without an indefinite blocking call: dispatch any
        // pending events with a short bounded wait, then redraw whatever
        // surface asked for it.
        if dispatch_until(
            &conn,
            &mut event_queue,
            &mut state,
            Duration::from_millis(50),
        )? {
            // events arrived; possibly nothing visible changed, but cheaper
            // than skipping a redraw.
        }
        // Redraw surfaces that requested it.
        for i in 0..state.overlays.len() {
            if state.overlays[i].needs_redraw && state.overlays[i].configured {
                render_overlay(i, &shm, &qh, &mut state);
            }
        }
    }

    // Build the outcome.
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
    // Cleanup. Order matters here:
    //   1. Tear down every per-overlay buffer + pool + layer surface so the
    //      compositor knows the overlay is gone and removes it.
    //   2. Release pointer / keyboard.
    //   3. Round-trip once so the destroy requests are flushed and acked
    //      before we drop the connection. Without this the compositor can
    //      legitimately keep showing the layer surface for a few frames
    //      *after* the program has called the post-action (clipboard /
    //      file save), which the user perceives as "the app didn't close".
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

// ---------------------------------------------------------------------------
// Dispatching with timeout
// ---------------------------------------------------------------------------

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
    match poll(&mut fds, timeout.as_millis() as i32) {
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

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub(crate) struct State {
    running: bool,
    outcome: Option<Outcome>,
    action: PostAction,
    canvas: Canvas,
    runtime_mode: SelectorMode,
    config: crate::selector::Config,
    background: Option<CapImage>,
    overlays: Vec<Overlay>,
    /// `wl_output` info collected from registry + Geometry / Mode events.
    /// Keyed by the wl_registry global name. Used to match each
    /// `sss_capture::Monitor` to the correct `WlOutput` *by position*,
    /// rather than by array index (which produces miswired overlays on
    /// multi-monitor setups where the sss_capture order and the wayland
    /// global order disagree — e.g. niri with 4 outputs, one rotated).
    output_infos: HashMap<u32, WlOutputInfo>,
    active_overlay: Option<usize>,
    pointer_pos_local: FPoint,
    pointer_pos_global: FPoint,
    mods: ModState,
    /// The user pressed `P` — the next left click samples the background
    /// pixel under the cursor and applies it as the active stroke
    /// colour. Reset back to `false` after the sample (or on Esc).
    pipette_pending: bool,
    /// User-toggled HUD overlay that magnifies the pixels under the
    /// cursor. Toggled with `M`.
    magnifier_on: bool,
    /// Snap drag endpoints to a 10-px grid and to existing shape
    /// vertices within tolerance. Toggled with `G`.
    snap_on: bool,
    /// HSV custom-colour popup. When `Some`, the renderer draws the
    /// picker beneath the toolbar and the click handler routes to it.
    picker: Option<HsvPicker>,
    /// While the user holds the left button after clicking inside the
    /// picker, motion events keep updating the same region (Hue strip
    /// or SV square) so dragging through the gradient feels live.
    picker_drag: Option<HsvHit>,
    /// Stroke-width slider popup.
    width_popup: Option<WidthPopup>,
    /// Snap-step slider popup — its own instance, intentionally not a
    /// reskin of the width popup. It has its own renderer + hit-test
    /// so the dotted preview can grow independently and a tweak to
    /// the stroke slider can't accidentally bleed in.
    snap_popup: Option<SnapPopup>,
    /// Bitmap of vertices the snap engine considered a hit during the
    /// last `pointer-move`. Only used as a visual cue.
    snap_marker: Option<FPoint>,
    /// Snap-grid spacing in screen pixels. The "snap" toolbar button
    /// opens the same slider popup as the stroke width for editing this.
    snap_step: f32,
    /// Persistent stroke colour kept *outside* the active tool so it
    /// survives tool switches. The active tool is reseated to this
    /// colour every time the tool changes.
    current_color: crate::color::Color,
    /// Same idea for stroke width.
    current_width: f32,
    /// Persistent fill colour (used for closed-shape variants that draw
    /// a filled interior).
    current_fill: Option<crate::color::Color>,
    /// When the user opens the colour popup via hover, the renderer
    /// shows the same `HsvPicker` until the cursor leaves the swatch.
    color_hover_at: Option<u64>,
    /// Likewise for the width slider — opened by hovering the width chip.
    width_hover: bool,
    /// Quick-access radial menu (opened with right-click). When `Some`
    /// the renderer paints a ring of preset colour / width pills and
    /// the click handler routes to it.
    radial: Option<RadialMenu>,
    /// Drag/resize/rotate gizmo for the selected shape. Activated by
    /// the new gizmo button in the selection decor.
    transform_drag: Option<TransformDrag>,
    /// Per-pointer cursor management (lazy, lives next to the wl_pointer).
    cursor_ctx: Option<super::cursor::CursorContext>,
    /// Cached so the cursor module can call `wl_compositor.create_surface`
    /// without us threading it through every call.
    compositor: Option<WlCompositor>,
    pointer: Option<WlPointer>,
    /// QueueHandle for spawning cursor / helper surfaces from inside a
    /// `Dispatch` callback.
    qh: Option<QueueHandle<State>>,
}

struct WlOutputInfo {
    wl_output: WlOutput,
    x: i32,
    y: i32,
    /// Physical pixel size announced by `wl_output.mode`.
    mode_w: i32,
    mode_h: i32,
    /// Compositor scale factor (wl_output.scale event).
    scale: i32,
    /// Wayland transform enum value (0=Normal, 1=90, 2=180, 3=270, …).
    transform: i32,
    /// True once `wl_output.done` arrived — i.e. all geometry events have
    /// landed and this entry is safe to read.
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
    /// Pool of SHM buffers — at least 2 for true double-buffering. Each
    /// `BusyFlag` flips to `true` when we attach the buffer and back to
    /// `false` when the compositor sends `wl_buffer.release`. Render picks
    /// the first non-busy buffer (or allocates a new one if both are still
    /// in flight).
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

// ---------------------------------------------------------------------------
// Rendering — paint pixels into the surface's wl_shm buffer.
// ---------------------------------------------------------------------------

fn render_overlay(idx: usize, shm: &WlShm, qh: &QueueHandle<State>, state: &mut State) {
    use std::sync::atomic::Ordering;
    let (w, h) = state.overlays[idx].size;
    if w == 0 || h == 0 {
        return;
    }
    let stride = w as usize * 4;
    let size = stride * h as usize;

    // Drop any buffers whose size doesn't match the current surface size.
    state.overlays[idx].buffers.retain(|b| b.size == size);

    // Pick a buffer the compositor isn't currently reading from.
    let buf_idx = state.overlays[idx]
        .buffers
        .iter()
        .position(|b| !b.busy.load(Ordering::Acquire));
    let buf_idx = match buf_idx {
        Some(i) => i,
        None => {
            // Both buffers in flight — allocate a new one (capped at 3 to
            // avoid unbounded growth on a stuck compositor).
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
            let _ = file; // pool dup'd the fd
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

    // Paint. We take the OverlayBuffer out of the Vec to satisfy the borrow
    // checker (paint needs &State; the mmap lives inside state.overlays).
    let mut buf = state.overlays[idx].buffers.swap_remove(buf_idx);
    paint(idx, w, h, &mut buf.mmap, state);
    // Mark busy *before* we attach + commit.
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

/// Paint XRGB8888 pixels straight into the mmap. Layout matches what
/// `wl_shm::Format::Xrgb8888` advertises: bytes `[B, G, R, X]` little
/// endian.
fn paint(idx: usize, w: u32, h: u32, mmap: &mut MmapMut, state: &State) {
    let mon_bounds = state.overlays[idx].monitor.bounds();
    let bytes = mmap.as_mut();

    // 1) Build the background + shapes in an RGBA scratch image, then blit
    //    that into the SHM buffer (RGBA → BGRX). Doing this in RGBA space
    //    lets us reuse `composite::flatten_with_preview` so the user sees
    //    every shape — including the in-flight drag preview and any
    //    pending text — *live* as they paint.
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
    // Apply every shape (committed + in-flight preview + pending text).
    crate::render::composite::flatten_with_preview(
        &mut rgba,
        &state.canvas,
        (mon_bounds.x(), mon_bounds.y()),
    );

    // 2) Copy the RGBA scratch into the SHM buffer as BGRX in one tight loop.
    let raw = rgba.as_raw();
    for i in 0..(w * h) as usize {
        let s = i * 4;
        bytes[s] = raw[s + 2]; // B
        bytes[s + 1] = raw[s + 1]; // G
        bytes[s + 2] = raw[s]; // R
        bytes[s + 3] = 0xff; // X
    }

    // 2) Subtle dim outside the active region.
    if let Some(region) = state.canvas.region() {
        // Coordinates in monitor-local pixels.
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
        if r_w > 0 && r_h > 0 {
            // Darken everything outside (r_x, r_y, r_w, r_h).
            for y in 0..h as i32 {
                for x in 0..w as i32 {
                    let inside = x >= r_x && x < r_x + r_w && y >= r_y && y < r_y + r_h;
                    if inside {
                        continue;
                    }
                    let d = (y as u32 * w * 4 + x as u32 * 4) as usize;
                    bytes[d] = bytes[d].saturating_sub(80);
                    bytes[d + 1] = bytes[d + 1].saturating_sub(80);
                    bytes[d + 2] = bytes[d + 2].saturating_sub(80);
                }
            }
            // Draw the rectangle outline in cyan.
            for x in 0..r_w {
                paint_pixel(bytes, w, h, (r_x + x) as u32, r_y as u32, [255, 255, 255]);
                paint_pixel(
                    bytes,
                    w,
                    h,
                    (r_x + x) as u32,
                    (r_y + r_h - 1) as u32,
                    [255, 255, 255],
                );
            }
            for y in 0..r_h {
                paint_pixel(bytes, w, h, r_x as u32, (r_y + y) as u32, [255, 255, 255]);
                paint_pixel(
                    bytes,
                    w,
                    h,
                    (r_x + r_w - 1) as u32,
                    (r_y + y) as u32,
                    [255, 255, 255],
                );
            }
        }
    }

    // 2.5) Snap grid (faint dots) — only visible while snap mode is on
    //      AND there's an active editing region. Painting the grid over
    //      the dimmed outside area is just visual noise; clipping to the
    //      region keeps it focused on the canvas the user is editing.
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
                    // The snapper rounds against *global* coords, so the
                    // visual grid has to anchor to the same origin —
                    // otherwise the dots and the actual snap targets
                    // drift apart by `(mon.x() % step, mon.y() % step)`.
                    (mon_bounds.x(), mon_bounds.y()),
                );
            }
        }
    }

    // 3) Selection decoration — dashed bounding box around the shape the
    //    user has picked with the Pointer tool, plus a tiny "delete"
    //    button. Drawn after the canvas but before the toolbar so the
    //    toolbar can sit on top.
    draw_selection_decor(bytes, w, h, idx, state);

    // 4) Floating toolbar — drawn on whichever overlay currently hosts the
    //    selection, anchored *to the selection rectangle*. For Monitor /
    //    Window modes (no rectangle yet) we anchor to the cursor overlay's
    //    top-center so the user always has access to Confirm / Cancel.
    let main_layout = state.toolbar_layout_for(idx);
    let side_layout = state.side_toolbar_layout_for(idx);
    if let Some(layout) = main_layout.as_ref() {
        layout.draw(bytes, w, h, state);
    }
    if let Some(layout) = side_layout.as_ref() {
        layout.draw(bytes, w, h, state);
    }
    if main_layout.is_none() && state.overlays[idx].pointer_inside {
        // Fall-back instruction line when there's no toolbar yet.
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

    // 5) HSV colour picker popup (opt-in via the "MORE" button).
    if let Some(picker) = state.picker.as_ref() {
        if picker.overlay_idx == idx {
            draw_hsv_picker(bytes, w, h, picker);
        }
    }

    // 5.4) Radial quick-access menu (right-click). Drawn before the
    //      width popup so the slider sits on top if both somehow stack.
    if let Some(r) = state.radial.as_ref() {
        if r.overlay_idx == idx {
            let palette = &state.config.palette.color_palette;
            let mon = state.overlays[idx].monitor.bounds();
            let lx = state.pointer_pos_global.x as i32 - mon.x();
            let ly = state.pointer_pos_global.y as i32 - mon.y();
            let hover_c = r.slot_color(palette.len(), lx, ly);
            let hover_w = r.slot_width(lx, ly, palette.len());
            draw_radial_menu(bytes, w, h, r, palette, hover_c, hover_w);
        }
    }

    // 5.5) Width slider popup.
    if let Some(wp) = state.width_popup.as_ref() {
        if wp.overlay_idx == idx {
            draw_width_popup(bytes, w, h, wp, state.active_tool_width());
        }
    }

    // 5.55) Snap-step slider popup — its own widget.
    if let Some(sp) = state.snap_popup.as_ref() {
        if sp.overlay_idx == idx {
            draw_snap_popup(bytes, w, h, sp, state.snap_step);
        }
    }

    // 5.6) Snap-target marker (a small white cross that shows up at the
    //      snapped point so the user can tell the grid/vertex caught it).
    if state.snap_on {
        if let Some(p) = state.snap_marker {
            let mon = state.overlays[idx].monitor.bounds();
            let lx = p.x as i32 - mon.x();
            let ly = p.y as i32 - mon.y();
            for d in -5..=5 {
                paint_pixel(bytes, w, h, (lx + d) as u32, ly as u32, [255, 255, 255]);
                paint_pixel(bytes, w, h, lx as u32, (ly + d) as u32, [255, 255, 255]);
            }
            // 1px outline so it's visible on white backgrounds too.
            for d in -5..=5 {
                paint_pixel(bytes, w, h, (lx + d) as u32, (ly - 1) as u32, [0, 0, 0]);
                paint_pixel(bytes, w, h, (lx + d) as u32, (ly + 1) as u32, [0, 0, 0]);
                paint_pixel(bytes, w, h, (lx - 1) as u32, (ly + d) as u32, [0, 0, 0]);
                paint_pixel(bytes, w, h, (lx + 1) as u32, (ly + d) as u32, [0, 0, 0]);
            }
        }
    }

    // 5.7) Constrain guides removed — they didn't help in practice; the
    //      Shift modifier is still honoured by the canvas, we just don't
    //      paint the reference rays anymore.

    // 6) Magnifier HUD — drawn last so it sits above every other overlay
    //    element (toolbar, picker, etc.) when the user toggles it on.
    draw_magnifier(bytes, w, h, idx, state);

    // 7) Pipette / snap mode hint, top-left of the active overlay.
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
    bytes[d] = rgb[2]; // B
    bytes[d + 1] = rgb[1]; // G
    bytes[d + 2] = rgb[0]; // R
    bytes[d + 3] = 0xff;
}

// (Floating toolbar implementation lives at the bottom of this file.)

/// Draw a string with the 5x7 bitmap font we already embedded in
/// `composite::glyph_bits`. We can't reuse `composite::draw_text` directly
/// because it operates on `image::RgbaImage`; this version writes straight
/// to the `wl_shm` BGRX buffer.
fn draw_text(bytes: &mut [u8], w: u32, h: u32, x: u32, y: u32, text: &str, rgb: [u8; 3]) {
    draw_text_sized(bytes, w, h, x, y, text, rgb, 13.0);
}

/// Draw `text` with Hack-Regular at `px` pixel size, anchored at top-left
/// `(x, y)` in monitor-local coordinates. Per-glyph coverage is alpha-
/// blended against the existing BGRX bytes so anti-aliased edges look
/// clean over both light and dark backgrounds.
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
                // Whitespace or a missing glyph — advance and continue.
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

// ---------------------------------------------------------------------------
// Dispatch impls
// ---------------------------------------------------------------------------

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
                    // Remember the serial; every subsequent set_cursor
                    // request has to echo it back to the compositor.
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
                    // Picker drag: dragging through the hue strip or the
                    // SV square updates the colour live without falling
                    // through into the canvas.
                    if let Some(hit) = state.picker_drag {
                        if let Some(p) = state.picker.as_ref() {
                            let mon = state.overlays[i].monitor.bounds();
                            let lx = state.pointer_pos_global.x as i32 - mon.x();
                            let ly = state.pointer_pos_global.y as i32 - mon.y();
                            // Allow dragging slightly outside the bounds so
                            // the marker can be pinned to an edge.
                            let _ = p;
                            state.apply_picker_hit(hit, lx, ly);
                            mark_all_redraw(state);
                            return;
                        }
                    }
                    // Width-slider drag: same idea — keep updating value.
                    if state.width_popup.as_ref().is_some_and(|w| w.dragging) {
                        let mon = state.overlays[i].monitor.bounds();
                        let lx = state.pointer_pos_global.x as i32 - mon.x();
                        let ly = state.pointer_pos_global.y as i32 - mon.y();
                        state.apply_width_slider(lx, ly);
                        mark_all_redraw(state);
                        return;
                    }
                    // Snap-slider drag.
                    if state.snap_popup.as_ref().is_some_and(|w| w.dragging) {
                        let mon = state.overlays[i].monitor.bounds();
                        let lx = state.pointer_pos_global.x as i32 - mon.x();
                        let ly = state.pointer_pos_global.y as i32 - mon.y();
                        state.apply_snap_slider(lx, ly);
                        mark_all_redraw(state);
                        return;
                    }
                    // Snap the global pointer to the nearest 10-px grid +
                    // shape vertex when the user has the SNAP toggle on.
                    // The raw position stays untouched in
                    // `state.pointer_pos_global` so the cursor visual
                    // tracks the real mouse position; only the position
                    // fed to the canvas is snapped.
                    let (p, snap_hit) = if state.snap_on {
                        let snapped =
                            snap_point(&state.canvas, state.pointer_pos_global, state.snap_step);
                        let dx = snapped.x - state.pointer_pos_global.x;
                        let dy = snapped.y - state.pointer_pos_global.y;
                        // Only show the marker when the snap actually moved
                        // the point — otherwise the cross would flash
                        // constantly under the cursor on every motion.
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
                    // Cursor swap is cheap (no-op when the name didn't
                    // change) so it's safe on every motion event.
                    refresh_cursor(state);
                    // *Only* repaint when a drag is in progress. Hover-only
                    // mouse movement changes nothing visible, and triggering
                    // a full SHM redraw on every motion event was the source
                    // of the user-reported flicker. When dragging we mark
                    // *every* overlay dirty: the region rectangle, the dim
                    // mask and any cross-monitor stroke have to update on
                    // every screen, not just the one the cursor is on.
                    // Active transform-gizmo drag — translate pointer
                    // delta into a scale factor / rotation angle and
                    // commit it back onto the selected shape. Eats the
                    // motion so the canvas's normal drag handler doesn't
                    // also fire.
                    if state.transform_drag.is_some() {
                        state.apply_transform_drag();
                        mark_all_redraw(state);
                        return;
                    }
                    // Hover-driven popup management: when the pointer
                    // sits on the persistent colour swatch / width chip /
                    // snap-step chip we open the matching popup; when it
                    // leaves both the chip and the popup we close any
                    // hover-opened popup. Click-pinned popups are left
                    // alone — those stay open until clicked away.
                    state.update_hover_popups();
                    // Repaint on motion when a drag is in progress *or*
                    // when a cursor-tracking HUD is on (magnifier / pipette
                    // overlay). Pure hover is otherwise skipped because a
                    // full SHM redraw on every motion event is what caused
                    // the earlier flicker.
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
                // Right-click (BTN_RIGHT = 0x111):
                //   * commits the in-flight polygon when one's in progress
                //   * otherwise opens the quick-access radial menu under
                //     the cursor (colour swatches + width presets).
                if button == 0x111 {
                    if matches!(btn_state, WEnum::Value(ButtonState::Pressed)) {
                        if state.canvas.is_drawing_polygon() {
                            state.canvas.commit_polygon();
                            mark_all_redraw(state);
                        } else if state.radial.is_some() {
                            // Toggle off when already open.
                            state.radial = None;
                            mark_all_redraw(state);
                        } else if let Some(i) = state.active_overlay {
                            let mon = state.overlays[i].monitor.bounds();
                            let lx = state.pointer_pos_global.x as i32 - mon.x();
                            let ly = state.pointer_pos_global.y as i32 - mon.y();
                            // Compute the menu rect for the current
                            // palette and clamp it so it stays fully on
                            // the overlay even when the cursor is near
                            // a corner.
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
                    // End any active popup drag regardless of where the
                    // release lands — releasing the button is the user's
                    // way of locking in the value.
                    state.picker_drag = None;
                    if let Some(wp) = state.width_popup.as_mut() {
                        wp.dragging = false;
                    }
                    if let Some(sp) = state.snap_popup.as_mut() {
                        sp.dragging = false;
                    }
                    // Commit the gizmo drag: leave the shape in its
                    // final transformed state and snapshot the history.
                    if state.transform_drag.is_some() {
                        state.transform_drag = None;
                        state.canvas.snapshot_history();
                        mark_all_redraw(state);
                    }
                }
                if let Some(i) = state.active_overlay {
                    // If the click landed on a toolbar button, treat it as
                    // a toolbar action and *don't* feed it to the canvas
                    // (otherwise we'd start a stray brush stroke on the
                    // toolbar's own pixels).
                    if pressed {
                        // Pipette: when armed, the next left click samples
                        // the background pixel under the cursor and uses
                        // it as the new stroke colour. Does *not* commit
                        // any shape.
                        if state.pipette_pending {
                            sample_pipette(state);
                            state.pipette_pending = false;
                            mark_all_redraw(state);
                            return;
                        }
                        // Quick-access menu: rectangular buttons make
                        // slot hit-tests reliable. Clicks on a colour or
                        // width cell commit + close; clicks anywhere
                        // else inside the panel are eaten (so they don't
                        // start a stroke through the menu).
                        if state.radial.is_some() {
                            let mon = state.overlays[i].monitor.bounds();
                            let lx = state.pointer_pos_global.x as i32 - mon.x();
                            let ly = state.pointer_pos_global.y as i32 - mon.y();
                            let palette_len = state.config.palette.color_palette.len();
                            let r = state.radial.as_ref().unwrap();
                            let inside = r.contains(palette_len, lx, ly);
                            let color_slot = r.slot_color(palette_len, lx, ly);
                            let width_slot = r.slot_width(lx, ly, palette_len);
                            if let Some(slot) = color_slot {
                                let c = state.config.palette.color_palette[slot];
                                state.apply_pick_color(c);
                                state.radial = None;
                            } else if let Some(slot) = width_slot {
                                let w = RADIAL_WIDTHS[slot];
                                state.update_current_width(w);
                                state.radial = None;
                            } else if !inside {
                                state.radial = None;
                            }
                            mark_all_redraw(state);
                            return;
                        }
                        // HSV colour picker — any click inside the
                        // popup's outer rect is consumed (so it doesn't
                        // fall through to the canvas and start a stroke),
                        // and clicks land on one of the interactive
                        // regions (Hue/Sv) apply a hue/sat-val change.
                        // Clicking also pins the picker so it stays open
                        // even when the cursor leaves the chip column.
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
                        // Stroke-width slider eats any click inside its
                        // outer rect, pins, and starts a drag if the
                        // click was on the slider track.
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
                        // Snap-step slider — separate widget, same idea.
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
                        // Selection action bar (raise / lower / gizmo /
                        // delete) takes precedence over the canvas's
                        // normal click handling so the small icons remain
                        // reachable even when the click would otherwise
                        // land on the shape itself.
                        if let Some(action) = pointer_on_selection_button(state) {
                            state.apply_button(action);
                            mark_all_redraw(state);
                            return;
                        }
                        // Gizmo handles: the bottom-right corner box
                        // starts a uniform scale, the top-centre pin
                        // starts a rotation. Two separate handles keep
                        // the two operations discoverable.
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
                    // Clicking somewhere that isn't a popup, chip, gizmo
                    // handle or selection button closes any pinned
                    // popups before the canvas sees the click. Keeps
                    // popups from sticking around after the user has
                    // moved on to drawing.
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
                // Vertical scroll over the width button steps the active
                // tool's stroke width — 1 unit per notch, 5 with Shift.
                if !matches!(axis, WEnum::Value(wl_pointer::Axis::VerticalScroll)) {
                    return;
                }
                if !state.pointer_on_width_button() {
                    return;
                }
                let step = if state.mods.shift { 5.0 } else { 1.0 };
                // The protocol sends `value` in surface units. Positive =
                // down (smaller width), negative = up (larger width).
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
    // We need a QueueHandle to create surfaces. We borrow it from the
    // pointer (proxies cache their queue assignment).
    let _ = (&comp, &ptr); // borrow-check appeasement
                           // The compositor / qh are static for the duration of the run. Build a
                           // throwaway QueueHandle by getting it from any tracked proxy.
                           // Wayland-client proxies don't expose their qh directly, so we go via
                           // the connection: cursor surfaces are attached to the data queue, and
                           // every dispatch path lives there.
                           // In practice we already passed `qh` when creating each overlay; we
                           // reach it via `pointer`'s queue using the convention that
                           // proxies created with the same `&qh` share that handle.
                           //
                           // The simplest path is: thread the QueueHandle into State.
                           //
                           // …but for the smallest diff today we delegate via the `dispatch` qh
                           // captured at startup. See: `State::qh_for_cursor`.
    if let Some(qh) = state.qh.as_ref() {
        ctx.apply(&ptr, &comp, qh, desired);
    }
}

fn update_pointer(state: &mut State, overlay_idx: usize, sx: f64, sy: f64) {
    let mon = state.overlays[overlay_idx].monitor.bounds();
    state.pointer_pos_local = FPoint::new(sx as f32, sy as f32);
    state.pointer_pos_global = FPoint::new(mon.x() as f32 + sx as f32, mon.y() as f32 + sy as f32);
}

// evdev keycodes the protocol delivers (Linux input event codes). We skip
// xkbcommon for now — it adds a hard `-lxkbcommon` linker requirement and
// for an overlay's handful of shortcuts the layout-independent codes are
// good enough. (Layouts where C/S aren't on the AZERTY-equivalent of QWERTY
// `C`/`S` keys will need the xkbcommon path later.)
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
            wl_keyboard::Event::Keymap { .. } => {
                // We deliberately ignore the keymap and use raw evdev
                // keycodes for the small set of shortcuts the overlay
                // recognises. See the `ev::*` constants above.
            }
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                // wl_keyboard.modifiers gives us the xkb modifier mask,
                // but we just need the rough flags. Reading directly from
                // pressed-down keys (see Key event below) is more robust
                // for the overlay's needs. As a backup we still parse the
                // mask: bit 0 = Shift, bit 2 = Ctrl, bit 3 = Alt, bit 6 =
                // Super (default xkb-default layout).
                state.mods.shift = (mods_depressed & 0x01) != 0;
                state.mods.ctrl = (mods_depressed & 0x04) != 0;
                state.mods.alt = (mods_depressed & 0x08) != 0;
                state.mods.meta = (mods_depressed & 0x40) != 0;
                // Forward Shift to the canvas so two-point drags honour
                // the constrain modifier (square / circle / 45° line).
                state.canvas.set_constrain(state.mods.shift);
            }
            wl_keyboard::Event::Key {
                key,
                state: key_state,
                ..
            } => {
                let pressed = matches!(key_state, WEnum::Value(KeyState::Pressed));
                // Track modifier press/release locally too — the
                // `Modifiers` event sometimes lags behind a chord.
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
                            // Discard the in-flight text but keep the overlay open.
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
                            // Commit the text, stay in the overlay.
                            state.canvas.handle(CanvasEvent::TextCommit);
                            mark_all_redraw(state);
                        } else if state.canvas.is_drawing_polygon() {
                            // Close + commit the polygon, stay in the
                            // overlay for further edits.
                            state.canvas.commit_polygon();
                            mark_all_redraw(state);
                        } else {
                            confirm(state);
                        }
                    }
                    // Ctrl+Shift+Backspace clears the canvas. The single
                    // backspace stays bound to Text-mode backspace so the
                    // text tool keeps working.
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
                    // Layer order — matches the convention used by Figma,
                    // Photoshop, Inkscape (Ctrl+] forwards, Ctrl+[ backwards).
                    // Shift bumps to top / bottom.
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
                    // Pipette — next click samples the background pixel
                    // under the cursor as the new stroke colour. Esc
                    // earlier in this match already cancels it.
                    ev::P if !state.mods.ctrl => {
                        state.pipette_pending = !state.pipette_pending;
                        mark_all_redraw(state);
                    }
                    // Magnifier HUD.
                    ev::M if !state.mods.ctrl => {
                        state.magnifier_on = !state.magnifier_on;
                        mark_all_redraw(state);
                    }
                    // Grid + vertex snapping.
                    ev::G if !state.mods.ctrl => {
                        state.snap_on = !state.snap_on;
                        mark_all_redraw(state);
                    }
                    // Scale the selected shape — `=` (with shift = `+`) makes
                    // it bigger, `-` makes it smaller. 10% steps; with Shift
                    // we bump to 25% for coarse adjustments.
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
                    // Rotate the selected shape. `,` rotates CCW, `.` CW.
                    // 5° steps by default; 45° with Shift.
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
            // Pick the monitor under the pointer.
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

// Stateless proxies — no events we care about.
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
            // Compositor finished reading — buffer is safe to overwrite.
            busy.store(false, Ordering::Release);
        }
    }
}
delegate_noop!(State: ignore ZwlrLayerShellV1);

// Silence: HashMap of monitor-by-name kept around for future use.
#[allow(dead_code)]
fn _silence_map(_: HashMap<u32, ()>) {}

// Silence: Write trait kept around for any future SHM debug dump.
#[allow(dead_code)]
fn _silence_write(_: &mut dyn Write) {}

// Silence Instant import (kept for future redraw throttling).
#[allow(dead_code)]
fn _silence_instant() -> Instant {
    Instant::now()
}

// ============================================================================
// Floating toolbar — CPU-rendered, anchored to the live selection.
// ============================================================================
//
// The toolbar is *anchored* to whatever's currently selected:
//
//   * Area / AnyOf mode with a rectangle ≥ 2×2 → toolbar floats just above
//     the rectangle (or just below it when there's no room above), centred
//     horizontally on the rectangle. Updates on every drag event.
//   * Monitor mode → top-centre of the overlay under the cursor.
//   * Window mode → top-centre of the overlay under the cursor.
//   * No selection yet → no toolbar (just the hint line).
//
// Buttons are plain coloured rectangles painted directly into the wl_shm
// buffer; hit-tests are done in monitor-local pixel coordinates against
// the same rects. No GPU, no widget toolkit.

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
    /// Same as `SelectTool` but also turns fill mode on so the user gets
    /// the filled variant of a closed shape.
    SelectToolFilled(usize),
    /// Wipe every committed shape from the canvas.
    ClearAll,
    /// Open / close the HSV custom colour picker popup.
    TogglePicker,
    /// Open / close the stroke-width slider popup.
    ToggleWidthPopup,
    /// Open / close the snap-step slider popup (reuses the same widget).
    ToggleSnapPopup,
    /// Raise the selected shape one layer.
    RaiseSelected,
    /// Lower the selected shape one layer.
    LowerSelected,
    /// Toggle the snap-to-grid mode.
    ToggleSnap,
    /// Toggle the magnifier HUD.
    ToggleMagnifier,
    /// Arm the pipette so the next click samples a colour.
    TogglePipette,
    DeleteSelected,
    Undo,
    Redo,
    Cancel,
    Confirm,
    Copy,
    Save,
}

/// One of the small CPU-drawn icons rendered on tool / action buttons.
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
    /// Solid square painted with the current colour (used by the
    /// persistent colour-swatch button).
    ColorSwatch,
    /// Tiny clear-all icon.
    Clear,
    /// Pipette / eyedropper icon.
    Pipette,
    /// Snap-to-grid icon (the grid).
    Snap,
    /// Magnifier (a lens with a handle).
    Magnifier,
    /// Raise (front) chevron.
    Raise,
    /// Lower (back) chevron.
    Lower,
    /// Trash can — used for "delete selected".
    Trash,
    /// Diagonal arrows — scale handle in the selection decor.
    GizmoScale,
    /// Curved arrow — rotate handle in the selection decor.
    GizmoRotate,
}

pub(crate) struct ToolbarButton {
    /// Rect in MONITOR-LOCAL pixels (origin top-left of the overlay).
    pub rect: (i32, i32, u32, u32),
    pub action: ButtonAction,
    /// Optional icon — drawn centered inside the button.
    pub icon: Option<ToolbarIcon>,
    /// Optional text label — drawn centered when no icon is set.
    /// `Cow` so callers can supply either a `&'static str` constant or a
    /// dynamically formatted string (e.g. the stroke-width readout).
    pub label: std::borrow::Cow<'static, str>,
    pub tint: Option<[u8; 3]>,
    pub active: bool,
}

pub(crate) struct ToolbarLayout {
    /// Monitor-local rect for the toolbar background.
    bg: (i32, i32, u32, u32),
    buttons: Vec<ToolbarButton>,
}

impl ToolbarLayout {
    /// Rect of the button whose action matches `action`, if any. The
    /// hover engine uses this to compute the chip↔popup corridor.
    fn button_rect_for(&self, action: ButtonAction) -> Option<(i32, i32, u32, u32)> {
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

    fn draw(&self, bytes: &mut [u8], w: u32, h: u32, _state: &State) {
        // Background panel.
        fill_rect_bytes(bytes, w, h, self.bg, [22, 22, 24]);
        // Subtle 1px border.
        outline_rect(bytes, w, h, self.bg, [80, 80, 84]);

        for b in &self.buttons {
            let (rx, ry, rw, rh) = b.rect;
            let fill = if b.active {
                [60, 110, 200]
            } else {
                b.tint.unwrap_or([42, 42, 46])
            };
            fill_rect_bytes(bytes, w, h, (rx, ry, rw, rh), fill);
            outline_rect(
                bytes,
                w,
                h,
                (rx, ry, rw, rh),
                if b.active {
                    [180, 220, 255]
                } else {
                    [70, 70, 76]
                },
            );
            // Prefer the icon over the text label when one is supplied.
            if let Some(icon) = b.icon {
                let cx = rx + rw as i32 / 2;
                let cy = ry + rh as i32 / 2;
                draw_icon(bytes, w, h, cx, cy, icon, [240, 240, 240]);
            } else if !b.label.is_empty() {
                // The chip uses the TTF renderer (`draw_text` →
                // Hack-Regular at 13 px), so we have to measure the
                // text in that font — not the old 5×7 bitmap. With
                // the bitmap-derived width the label drifted to the
                // left edge of the chip.
                let label = b.label.as_ref();
                let px = 13.0;
                let text_w = super::font::measure(label, px).ceil() as i32;
                // Approximate cap height — Hack at 13 px draws roughly
                // 9 px tall caps with a small ascent above the baseline.
                let text_h = (px * 0.75) as i32;
                let tx = rx + (rw as i32 - text_w) / 2;
                // draw_text anchors at the top-left of the glyph cell
                // (y is the *top* of the bounding box, not the baseline),
                // so the vertical centring is just (rh - text_h) / 2.
                let ty = ry + (rh as i32 - text_h) / 2;
                draw_text(
                    bytes,
                    w,
                    h,
                    tx.max(rx + 2) as u32,
                    ty.max(ry + 2) as u32,
                    label,
                    [240, 240, 240],
                );
            }
        }
    }
}

/// CPU-draw a 16×16-ish icon centered on (cx, cy). The shapes are kept
/// intentionally simple — straight lines, filled rects, a small circle —
/// so users can recognise the tool without needing a font pack on the
/// system.
fn draw_icon(bytes: &mut [u8], w: u32, h: u32, cx: i32, cy: i32, kind: ToolbarIcon, rgb: [u8; 3]) {
    // Try the user-supplied SVG first; fall back to the built-in geometry
    // when the placeholder is still empty or the SVG fails to parse.
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
            // Source-over against existing BGRX.
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
    let s = 7; // half-size of the icon bounding box
    match kind {
        ToolbarIcon::Pointer => {
            // Arrowhead pointing up-left.
            for i in 0..=s {
                paint_pixel(bytes, w, h, (cx - s + i) as u32, (cy - s + i) as u32, rgb);
            }
            for i in 0..=s {
                paint_pixel(bytes, w, h, (cx - s) as u32, (cy - s + i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx - s + i) as u32, (cy - s) as u32, rgb);
            }
            // Tail to bottom-right.
            for i in 0..=(s - 1) {
                paint_pixel(bytes, w, h, (cx + i / 2) as u32, (cy + i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + i / 2 + 1) as u32, (cy + i) as u32, rgb);
            }
        }
        ToolbarIcon::Brush => {
            // Diagonal "pencil" stroke.
            for i in -s..=s {
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy + i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + i + 1) as u32, (cy + i) as u32, rgb);
            }
            // A little nib at the bottom-right.
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
            // Diagonal line + arrowhead at the bottom-right.
            for i in -s..=s {
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy + i) as u32, rgb);
            }
            // Head: horizontal + vertical segments.
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
            // 4×4 checkerboard pattern.
            for y in -s..=s {
                for x in -s..=s {
                    if ((x + s) / 2 + (y + s) / 2) % 2 == 0 {
                        paint_pixel(bytes, w, h, (cx + x) as u32, (cy + y) as u32, rgb);
                    }
                }
            }
        }
        ToolbarIcon::Eraser => {
            // Big X.
            for i in -s..=s {
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy + i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy - i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + i + 1) as u32, (cy + i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy + i + 1) as u32, rgb);
            }
        }
        ToolbarIcon::Step => {
            // Circle outline + "1" in the middle.
            let r = s as f32;
            for t in 0..64 {
                let a = (t as f32 / 64.0) * std::f32::consts::TAU;
                let x = cx + (r * a.cos()).round() as i32;
                let y = cy + (r * a.sin()).round() as i32;
                paint_pixel(bytes, w, h, x as u32, y as u32, rgb);
            }
            // Tiny "1" rendered with the same bitmap font, centred.
            draw_text(bytes, w, h, (cx - 4) as u32, (cy - 6) as u32, "1", rgb);
        }
        ToolbarIcon::Text => {
            // Big "T".
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
            // Five-pointed regular pentagon outline.
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
            // Curved arrow ⤺.
            let r = (s - 1) as f32;
            for t in 16..56 {
                let a = (t as f32 / 64.0) * std::f32::consts::TAU;
                let x = cx + (r * a.cos()).round() as i32;
                let y = cy + (r * a.sin()).round() as i32;
                paint_pixel(bytes, w, h, x as u32, y as u32, rgb);
            }
            // Arrowhead at the end.
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
            // X (smaller than the eraser).
            for i in -(s - 1)..=(s - 1) {
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy + i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + i) as u32, (cy - i) as u32, rgb);
            }
        }
        ToolbarIcon::Confirm => {
            // Checkmark.
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
            // Two overlapping rectangles.
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
            // Floppy outline.
            for x in -s..=s {
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy - s) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy + s) as u32, rgb);
            }
            for y in -s..=s {
                paint_pixel(bytes, w, h, (cx - s) as u32, (cy + y) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + s) as u32, (cy + y) as u32, rgb);
            }
            // Top notch.
            for x in -(s - 3)..=(s - 3) {
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy - s + 3) as u32, rgb);
            }
            // Bottom label rectangle.
            for x in -(s - 3)..=(s - 3) {
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy + 1) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + x) as u32, (cy + s - 2) as u32, rgb);
            }
            for y in 1..=(s - 2) {
                paint_pixel(bytes, w, h, (cx - (s - 3)) as u32, (cy + y) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + (s - 3)) as u32, (cy + y) as u32, rgb);
            }
        }
        // Newly added variants — the SVG-rasterised version is what
        // ships in production; the fallback is just a small outlined
        // square so the chip is still visible if the SVG is missing.
        ToolbarIcon::GizmoScale => {
            // Diagonal arrow ↘↖ — a clear "drag the corner to resize" hint.
            for i in 0..=s {
                paint_pixel(bytes, w, h, (cx - s + i) as u32, (cy - s + i) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + s - i) as u32, (cy + s - i) as u32, rgb);
            }
            // Arrowheads on each tip.
            for d in 0..=3 {
                paint_pixel(bytes, w, h, (cx - s + d) as u32, (cy - s) as u32, rgb);
                paint_pixel(bytes, w, h, (cx - s) as u32, (cy - s + d) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + s - d) as u32, (cy + s) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + s) as u32, (cy + s - d) as u32, rgb);
            }
        }
        ToolbarIcon::GizmoRotate => {
            // 3/4 circle with an arrow tip — the standard "rotate" glyph.
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
            // Outlined diamond — distinct from the other simple icons so
            // unfilled SVG placeholders are still identifiable.
            for d in 0..s {
                paint_pixel(bytes, w, h, (cx + d) as u32, (cy - s + d) as u32, rgb);
                paint_pixel(bytes, w, h, (cx - d) as u32, (cy - s + d) as u32, rgb);
                paint_pixel(bytes, w, h, (cx + d) as u32, (cy + s - d) as u32, rgb);
                paint_pixel(bytes, w, h, (cx - d) as u32, (cy + s - d) as u32, rgb);
            }
        }
    }
}

/// Is `(px, py)` inside the literal chip-to-popup corridor: either of
/// the two rects (chip / popup) *or* the narrow vertical bridge that
/// joins them. Earlier the corridor was the bounding union of the two
/// rects, but that bounding box is too generous — for a chip on the
/// far left and a wide popup beneath it, the rect could swallow every
/// chip in between and keep the popup open while the user was hovering
/// completely unrelated buttons.
fn in_chip_popup_corridor(
    chip: (i32, i32, u32, u32),
    popup: (i32, i32, u32, u32),
    px: i32,
    py: i32,
    slack: i32,
) -> bool {
    if rect_contains(chip, px, py, slack) {
        return true;
    }
    if rect_contains(popup, px, py, slack) {
        return true;
    }
    // Bridge: a column with the chip's horizontal extent, spanning
    // the vertical gap between the chip and the popup (whichever side
    // the popup is on).
    let chip_bottom = chip.1 + chip.3 as i32;
    let popup_bottom = popup.1 + popup.3 as i32;
    let (by, bh) = if popup.1 >= chip_bottom {
        (chip_bottom, (popup.1 - chip_bottom).max(0))
    } else if chip.1 >= popup_bottom {
        (popup_bottom, (chip.1 - popup_bottom).max(0))
    } else {
        // Chip and popup overlap vertically — no bridge needed.
        (0, 0)
    };
    if bh > 0 {
        rect_contains((chip.0, by, chip.2, bh as u32), px, py, slack)
    } else {
        false
    }
}

/// `true` when `(px, py)` is inside `rect` expanded by `slack` pixels
/// on every side.
fn rect_contains(rect: (i32, i32, u32, u32), px: i32, py: i32, slack: i32) -> bool {
    let (x, y, w, h) = rect;
    px >= x - slack && px < x + w as i32 + slack && py >= y - slack && py < y + h as i32 + slack
}

/// `true` when two rectangles share any pixel. Open-rect convention
/// (right/bottom edges exclusive), same as the rest of the file.
fn rects_overlap(a: (i32, i32, u32, u32), b: (i32, i32, u32, u32)) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    !(ax + aw as i32 <= bx || bx + bw as i32 <= ax || ay + ah as i32 <= by || by + bh as i32 <= ay)
}

fn fill_rect_bytes(bytes: &mut [u8], w: u32, h: u32, rect: (i32, i32, u32, u32), rgb: [u8; 3]) {
    let (rx, ry, rw, rh) = rect;
    for y in ry.max(0)..(ry + rh as i32).min(h as i32) {
        for x in rx.max(0)..(rx + rw as i32).min(w as i32) {
            paint_pixel(bytes, w, h, x as u32, y as u32, rgb);
        }
    }
}

fn outline_rect(bytes: &mut [u8], w: u32, h: u32, rect: (i32, i32, u32, u32), rgb: [u8; 3]) {
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
    /// Build a fresh `ToolbarLayout` if a toolbar should be shown on
    /// overlay `idx`. None means: don't draw a toolbar on this overlay.
    pub(crate) fn toolbar_layout_for(&self, idx: usize) -> Option<ToolbarLayout> {
        if !self.config.toolbar {
            return None;
        }
        let mon = self.overlays[idx].monitor.bounds();

        // ---- pick the anchor rectangle (in *global* / virtual-desktop coords)
        let (anchor_rect, follow_region) = match (self.canvas.region(), self.runtime_mode) {
            (Some(r), SelectorMode::Area) | (Some(r), SelectorMode::AnyOf)
                if r.width() >= 2 && r.height() >= 2 =>
            {
                (r, true)
            }
            (_, SelectorMode::Monitor) | (_, SelectorMode::Window) => {
                // Show the toolbar on the overlay currently under the cursor.
                if self.active_overlay != Some(idx) {
                    return None;
                }
                (mon, false)
            }
            _ => return None,
        };

        // For Area mode the toolbar only shows on the overlay that *contains*
        // the selection (or the cursor overlay if the selection spans multiple).
        if follow_region {
            if let Some(inter) = mon.intersection(&anchor_rect) {
                // Pick the overlay with the largest intersection.
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

        // ---- build the button list
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
            // Outlined variant — active only when this tool is selected
            // AND fill mode is off. (For closed shapes the filled variant
            // takes over when fill is on; non-closed shapes always go
            // through this chip.)
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
            // Filled companion chip for closed shapes — active only when
            // *this* tool is selected AND fill mode is on, so the two
            // chips never both light up at once.
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

        // Action buttons — built without a capturing closure so we can
        // freely poke at `buttons` from anywhere in this loop.
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
        // Persistent colour swatch — clicking opens the HSV picker, and
        // hovering also opens it (auto-close on leave). The tint is the
        // current State.current_color so it reflects the active colour
        // regardless of which tool is selected.
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
        // Stroke-width chip — shows the persistent State.current_width,
        // clicking *or* hovering opens the slider popup.
        let w = self.current_width.round() as i32;
        buttons.push(ToolbarButton {
            rect: (0, 0, (TB_BTN_W + 4) as u32, TB_BTN_H as u32),
            action: ButtonAction::ToggleWidthPopup,
            icon: None,
            label: std::borrow::Cow::Owned(format!("{}px", w)),
            tint: None,
            active: self.width_popup.is_some() || self.width_hover,
        });
        // Hint toggles.
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
        // Snap-step chip — opens its own dedicated popup (separate
        // widget from the stroke-width slider).
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
        // The canvas-unrelated actions (undo / redo / clear / cancel /
        // copy / save / confirm) live on a separate side toolbar — see
        // [`State::side_toolbar_layout_for`]. Keeping them off the main
        // toolbar makes the editor row less overwhelming.
        let _ = make_action;

        // ---- compute total width
        let mut total_w: i32 = TB_PAD_X * 2;
        let mut prev_kind: Option<u8> = None;
        for b in &buttons {
            // Kind is used to insert separators between groups: tools (0),
            // colors (1), actions (2).
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

        // ---- pick toolbar position (monitor-local coordinates)
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
            // Last resort: pinned to the top of the monitor.
            8
        };
        tb_y = tb_y.max(8).min(mon_h - total_h - 8).max(8);

        // ---- assign per-button rects walking left to right
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

    /// Build the *side* toolbar layout for overlay `idx`. The side bar
    /// holds the canvas-unrelated actions (undo / redo / clear / cancel
    /// / copy / save / confirm) and stays glued to the active region
    /// (or the cursor overlay in Monitor / Window mode). It hugs the
    /// right edge by default and flips to the left when that doesn't
    /// fit, mirroring how the main toolbar picks above vs. below.
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
            // Same "biggest intersection wins" logic as the main toolbar.
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

        // Build the button list.
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
        let mut buttons: Vec<ToolbarButton> = Vec::new();
        buttons.push(make_action(ToolbarIcon::Undo, ButtonAction::Undo));
        buttons.push(make_action(ToolbarIcon::Redo, ButtonAction::Redo));
        buttons.push(ToolbarButton {
            rect: (0, 0, TB_BTN_W as u32, TB_BTN_H as u32),
            action: ButtonAction::ClearAll,
            icon: Some(ToolbarIcon::Clear),
            label: std::borrow::Cow::Borrowed(""),
            tint: None,
            active: false,
        });
        buttons.push(make_action(ToolbarIcon::Cancel, ButtonAction::Cancel));
        if self.config.show_copy {
            buttons.push(make_action(ToolbarIcon::Copy, ButtonAction::Copy));
        }
        if self.config.show_save {
            buttons.push(make_action(ToolbarIcon::Save, ButtonAction::Save));
        }
        buttons.push(make_action(ToolbarIcon::Confirm, ButtonAction::Confirm));

        let total_h = TB_PAD_Y * 2
            + buttons.len() as i32 * TB_BTN_H
            + (buttons.len() as i32 - 1).max(0) * TB_GAP;
        let total_w = TB_PAD_X * 2 + TB_BTN_W;

        // ---- pick toolbar position (monitor-local coordinates)
        let region_local_x = anchor_rect.x() - mon.x();
        let region_local_y = anchor_rect.y() - mon.y();
        let region_local_h = anchor_rect.height() as i32;
        let mon_w = mon.width() as i32;
        let mon_h = mon.height() as i32;

        // Vertical centring on the region.
        let mut tb_y = region_local_y + (region_local_h - total_h) / 2;
        tb_y = tb_y.max(8).min(mon_h - total_h - 8).max(8);

        // The side toolbar needs to clear the main toolbar — when the
        // region is small enough that the main bar extends past the
        // region's right edge horizontally, anchoring to the region
        // edge would have the two bars overlap. Use whichever right
        // edge is farther out.
        let main_bg = self.toolbar_layout_for(idx).map(|l| l.bg);
        let main_right = main_bg
            .as_ref()
            .map(|(x, _, w, _)| x + *w as i32)
            .unwrap_or(0);
        let main_left = main_bg.as_ref().map(|(x, _, _, _)| *x).unwrap_or(mon_w);
        let region_right = region_local_x + anchor_rect.width() as i32;
        let region_left = region_local_x;
        // Prefer the right side, fall back to the left.
        let right = region_right.max(main_right) + TB_GAP_FROM_REGION;
        let left = region_left.min(main_left) - total_w - TB_GAP_FROM_REGION;
        let mut tb_x = if right + total_w <= mon_w - 8 {
            right
        } else if left >= 8 {
            left
        } else {
            // Last resort: hug the right edge of the monitor.
            mon_w - total_w - 8
        };
        tb_x = tb_x.max(8).min(mon_w - total_w - 8).max(8);

        // If even after the horizontal clearance the side bar overlaps
        // the main bar vertically, shift tb_y so the two stack cleanly.
        if let Some(main) = main_bg {
            let side_rect = (tb_x, tb_y, total_w as u32, total_h as u32);
            if rects_overlap(side_rect, main) {
                // Pick the direction with more room.
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

        // Position each button.
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

    /// Translate a global pointer position to monitor-local coords on the
    /// given overlay.
    fn pointer_local(&self, idx: usize) -> (i32, i32) {
        let mon = self.overlays[idx].monitor.bounds();
        (
            (self.pointer_pos_global.x as i32) - mon.x(),
            (self.pointer_pos_global.y as i32) - mon.y(),
        )
    }

    /// True if the pointer is currently sitting on a button anywhere in
    /// either toolbar (main editor row or side action column).
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

    /// Whether the pointer is anywhere inside the bounds of either
    /// toolbar — used by `desired_cursor` so the cursor doesn't change
    /// to a draw glyph when the user is just navigating the chrome.
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

    /// Apply the pointer-delta of the active gizmo drag to the selected
    /// shape: replace it with a transformed clone of `original`. Called
    /// on every motion event while a drag is in progress.
    fn apply_transform_drag(&mut self) {
        let Some(id) = self.canvas.selected() else {
            return;
        };
        // Snap the pointer to the grid before computing the gizmo
        // delta whenever snap mode is on — otherwise scale/rotate move
        // pixel-by-pixel in spite of the user wanting grid alignment.
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

    /// Open/close hover-triggered popups based on the current pointer
    /// position. Click-pinned popups are left alone.
    ///
    /// To avoid flicker when moving the pointer from the chip down into
    /// its popup, the close-test uses a *corridor*: the bounding union
    /// of the triggering chip's rect and the popup's rect (plus a few
    /// pixels of slack). As long as the pointer is anywhere inside that
    /// rectangle the popup stays open.
    fn update_hover_popups(&mut self) {
        // While the magnifier, pipette HUD or snap marker are on, the
        // on-screen pointer reads have a "mini cursor" companion that
        // tracks the real pointer (the magnifier ring, the pipette
        // sample crosshair or the snap-target cross). Moving toward
        // any of those overlays makes the cursor pass through chips on
        // the way, which used to fire hover-open repeatedly. Suppress
        // hover-driven opens in those modes — the user can still click
        // to open popups explicitly. Pinned popups are untouched.
        if self.magnifier_on || self.pipette_pending || self.snap_on {
            return;
        }
        // If any popup is already pinned, hover does nothing — the
        // user has explicitly opted into "this popup stays". Hover
        // can't override that.
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

        // Chip rects used in the chip↔popup corridor tests below.
        let color_chip = layout.button_rect_for(ButtonAction::TogglePicker);
        let width_chip = layout.button_rect_for(ButtonAction::ToggleWidthPopup);
        let snap_chip = layout.button_rect_for(ButtonAction::ToggleSnapPopup);

        let on_color_chip = matches!(action, Some(ButtonAction::TogglePicker));
        let on_width_chip = matches!(action, Some(ButtonAction::ToggleWidthPopup));
        let on_snap_chip = matches!(action, Some(ButtonAction::ToggleSnapPopup));

        // ---- Color swatch hover → HSV picker -----------------------------
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

        // ---- Width chip hover → stroke slider ----------------------------
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

        // ---- Snap chip hover → snap-step slider --------------------------
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

    /// Three-state click toggle for the stroke-width slider chip:
    ///   * closed             → open + pin
    ///   * open (hover-only)  → pin
    ///   * open + pinned      → close
    /// Always closes the colour picker and the snap popup so there's
    /// only one editor popup visible at a time.
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

    /// Same idea for the snap-step slider — entirely independent state.
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

    /// Reseat the active tool's brush settings from the persistent
    /// per-session values held in `State` (current_color / current_width /
    /// current_fill). Called whenever the active tool changes or the
    /// persistent values are updated, so the tool always reflects the
    /// swatch + slider readouts shown in the toolbar.
    ///
    /// `current_fill` is propagated faithfully: `Some` turns fill on
    /// (closed shapes get the tint), `None` turns it off. The earlier
    /// version only pushed when it was `Some`, which meant once the
    /// user had clicked a Filled variant the fill state was sticky —
    /// clicking back to the outlined chip set fill to None on the
    /// canvas but `push_current_to_tool` immediately re-enabled it
    /// because `current_fill` was still `Some`. Every subsequent
    /// outlined-tool selection then drew filled. Fixing both paths
    /// (here + `SelectTool` clearing `current_fill`) keeps the toggle
    /// honest.
    fn push_current_to_tool(&mut self) {
        set_active_tool_color(&mut self.canvas.active_tool, self.current_color);
        set_active_tool_width(&mut self.canvas.active_tool, self.current_width);
        self.canvas.set_fill_color(self.current_fill);
    }

    /// Apply the action attached to a button click. Returns true when the
    /// caller should stop the click from reaching the canvas.
    fn apply_button(&mut self, action: ButtonAction) -> bool {
        match action {
            ButtonAction::SelectTool(i) => {
                if let Some(t) = self.config.palette.tools.get(i).cloned() {
                    self.canvas.set_tool(t);
                    // Outlined variants disable fill mode. Clear both
                    // the persistent state *and* the canvas's setting
                    // — clearing only the canvas wasn't enough because
                    // `push_current_to_tool` would re-apply
                    // `current_fill` and turn fill right back on.
                    self.current_fill = None;
                    self.push_current_to_tool();
                    mark_all_redraw(self);
                }
            }
            ButtonAction::SelectToolFilled(i) => {
                if let Some(t) = self.config.palette.tools.get(i).cloned() {
                    self.canvas.set_tool(t);
                    // Filled variant: track fill alongside stroke from the
                    // persistent colour. Both update together on future
                    // picker / pipette / palette clicks (see
                    // `apply_pick_color`).
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
                // Three-state toggle:
                //   * closed             → open + pin
                //   * open (hover-only)  → pin (so it stays put when the
                //                          cursor wanders off the chip)
                //   * open + pinned     → close
                if let Some(p) = self.picker.as_mut() {
                    if p.pinned {
                        self.picker = None;
                        self.picker_drag = None;
                    } else {
                        p.pinned = true;
                    }
                } else if let Some(idx) = self.active_overlay {
                    // Opening any popup closes the other one — only one
                    // editor popup should be visible at a time.
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

/// Resolve a popup origin (`x`, `y`) that sits just below the toolbar
/// for the given overlay. Returns `None` when no toolbar is visible on
/// this overlay (e.g. the cursor moved off-region) — callers should
/// fall back to a screen-centre anchor in that case.
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
        // Toolbar is at the bottom — flip the popup above it instead.
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

/// Icon for the *filled* variant of a closed-shape tool. Only meaningful
/// for the three shapes that have a closed interior.
fn filled_tool_icon(t: &crate::tool::Tool) -> ToolbarIcon {
    use crate::tool::Tool;
    match t {
        Tool::Rectangle(_) => ToolbarIcon::RectangleFilled,
        Tool::Ellipse(_) => ToolbarIcon::EllipseFilled,
        Tool::Polygon(_) => ToolbarIcon::PolygonFilled,
        // The other tools shouldn't reach this branch (we only push the
        // filled chip for closed shapes), but return the outlined icon
        // so we never panic if the call site changes.
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
        // Steps don't have a stroke per se; drive their disc radius
        // off the stroke chip too so the same slider scales them.
        // `radius = width * 4 + 4` keeps the historical default 14 px
        // close to where it used to be (default width 3 → radius 16).
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

// ============================================================================
// Selection decoration (Pointer tool)
// ============================================================================

const DELETE_BTN_W: u32 = 24;
const DELETE_BTN_H: u32 = 24;
const DELETE_BTN_OFFSET: i32 = 4;
const SEL_BTN_GAP: i32 = 4;

/// One of the small action buttons that sit above a selected shape: the
/// delete X, the raise/lower layer arrows and the transform gizmo. Click
/// hit-tests in `pointer_on_selection_button()` dispatch to the matching
/// `ButtonAction`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SelectionButton {
    pub rect: (i32, i32, u32, u32),
    pub action: ButtonAction,
    pub icon: ToolbarIcon,
    pub tint: [u8; 3],
}

/// Layout (in monitor-local pixels) of the selection-decor floating
/// toolbar. Returns `None` when there's nothing selected on this
/// overlay or the Pointer tool isn't active.
fn selection_decor_layout(
    overlay_idx: usize,
    state: &State,
) -> Option<(
    /*shape*/ (i32, i32, u32, u32),
    Vec<SelectionButton>,
    /*scale handle*/ (i32, i32, u32, u32),
    /*rotate handle*/ (i32, i32, u32, u32),
)> {
    if !matches!(state.canvas.active_tool, crate::tool::Tool::Pointer) {
        return None;
    }
    let sel_id = state.canvas.selected()?;
    let shape = state.canvas.shapes().iter().find(|s| s.id == sel_id)?;
    let bounds = shape.bounds();
    let mon = state.overlays[overlay_idx].monitor.bounds();

    // Selection decor (action bar, gizmo handles, dashed outline) must
    // only render on the overlay that actually contains the shape. The
    // earlier implementation computed local coords against *every*
    // overlay's origin, which produced phantom decorations on other
    // monitors whenever the shape's local coords happened to overlap
    // the corresponding screen position. Same class of bug as the
    // original blur-leak: we now pick the overlay with the largest
    // intersection and bail out elsewhere.
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

    // ---- Action bar (raise / lower / gizmo / delete) -------------------
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

    // Horizontal placement: right-align against the shape's right edge,
    // then clamp so it stays on the monitor.
    let mut bar_x = local_x + bounds.width() as i32 - total_w + DELETE_BTN_OFFSET;
    bar_x = bar_x.clamp(4, (mon_w - total_w - 4).max(4));

    // Vertical placement: try above the shape. If that's off-screen,
    // place below. Reserve room above for the rotate handle when the
    // gizmo is on so the two never overlap.
    let extra_for_rotate = DELETE_BTN_H as i32 + 8;
    let above = local_y - DELETE_BTN_H as i32 - DELETE_BTN_OFFSET - extra_for_rotate;
    let below = local_y + bounds.height() as i32 + DELETE_BTN_OFFSET;
    let mut bar_y = if above >= 4 {
        above
    } else if below + DELETE_BTN_H as i32 <= mon_h - 4 {
        below
    } else {
        // Last resort: pin to the top inside the shape.
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

    // ---- Gizmo handles -------------------------------------------------
    // Scale: bottom-right corner. Rotate: top-centre, on a short stem
    // *between* the shape and the action bar so it never overlaps the
    // delete / raise / lower icons.
    let scale_x = (local_x + bounds.width() as i32 - DELETE_BTN_W as i32 / 2)
        .clamp(4, (mon_w - DELETE_BTN_W as i32 - 4).max(4));
    let scale_y = (local_y + bounds.height() as i32 - DELETE_BTN_H as i32 / 2)
        .clamp(4, (mon_h - DELETE_BTN_H as i32 - 4).max(4));
    let scale_rect = (scale_x, scale_y, DELETE_BTN_W, DELETE_BTN_H);

    let rotate_x = (local_x + bounds.width() as i32 / 2 - DELETE_BTN_W as i32 / 2)
        .clamp(4, (mon_w - DELETE_BTN_W as i32 - 4).max(4));
    // Sit immediately above the shape, with the action bar already
    // pushed an extra row higher to make room.
    let rotate_y = (local_y - DELETE_BTN_H as i32 - DELETE_BTN_OFFSET).max(4);
    let rotate_rect = (rotate_x, rotate_y, DELETE_BTN_W, DELETE_BTN_H);

    Some((shape_rect, buttons, scale_rect, rotate_rect))
}

/// Paint the dashed selection outline on *every* overlay whose monitor
/// intersects the selected shape, so the bounding box reads as a
/// continuous frame even when the user drags the shape across a
/// monitor boundary. Interactive widgets (action bar, gizmo handles)
/// stay scoped to the owning overlay via `selection_decor_layout`.
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
    // The dashed outline shows on *every* monitor the shape spans —
    // even on overlays the shape only just clips. The interactive bits
    // (action bar, scale / rotate handles) stay on the owning monitor.
    draw_selection_outline_if_visible(bytes, w, h, idx, state);

    let (shape_rect, buttons, scale_rect, rotate_rect) = match selection_decor_layout(idx, state) {
        Some(r) => r,
        None => return,
    };

    // 2) Action buttons (raise / lower / gizmo / delete).
    for b in &buttons {
        fill_rect_bytes(bytes, w, h, b.rect, b.tint);
        outline_rect(bytes, w, h, b.rect, [255, 220, 220]);
        let cx = b.rect.0 + b.rect.2 as i32 / 2;
        let cy = b.rect.1 + b.rect.3 as i32 / 2;
        draw_icon(bytes, w, h, cx, cy, b.icon, [240, 240, 240]);
    }

    // 3) Scale handle (bottom-right) — drag to scale.
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
    // 4) Rotate handle (top-centre, on a short stem) — drag to rotate.
    let stem_x = rotate_rect.0 + rotate_rect.2 as i32 / 2;
    let stem_y0 = shape_rect.1; // top of the selected shape
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

fn draw_dashed_rect(bytes: &mut [u8], w: u32, h: u32, rect: (i32, i32, u32, u32), rgb: [u8; 3]) {
    let (rx, ry, rw, rh) = rect;
    if rw == 0 || rh == 0 {
        return;
    }
    let on = 6;
    let off = 4;
    let stride = on + off;
    // Top + bottom edges.
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
    // Left + right edges.
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

/// Hit-test the gizmo handles (bottom-right corner = scale, top-centre
/// pin = rotate). Returns the handle kind and the shape rect, or `None`
/// when the pointer isn't over either.
fn pointer_on_gizmo_handle(state: &State) -> Option<(GizmoHandle, (i32, i32, u32, u32))> {
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

/// Returns the `ButtonAction` for whichever button in the selection-decor
/// the pointer is hovering, if any.
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

/// Bresenham line for the fallback icon drawer. Cheap and pixely.
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

// ============================================================================
// HSV color picker popup
// ============================================================================

// HSV picker geometry:
//   ┌──────────────────────────┐    HSV_PICKER_W wide
//   │   Hue strip   (HUE_H)    │
//   ├──────────────────────────┤
//   │                          │
//   │   Saturation / Value     │
//   │   square    (SV_H)       │
//   │                          │
//   ├────────────┬─────────────┤
//   │ swatch     │  #RRGGBB    │   foot row
//   └────────────┴─────────────┘
const HSV_PICKER_W: i32 = 260;
const HSV_HUE_H: i32 = 18;
const HSV_GAP_Y: i32 = 6;
const HSV_SV_H: i32 = 140;
const HSV_FOOT_H: i32 = 22;
const HSV_PICKER_H: i32 = HSV_HUE_H + HSV_GAP_Y + HSV_SV_H + HSV_GAP_Y + HSV_FOOT_H;
#[allow(dead_code)]
const HSV_PICKER_GAP: i32 = 8;

pub(crate) struct HsvPicker {
    /// Current hue in [0, 1).
    pub hue: f32,
    /// Current saturation in [0, 1].
    pub sat: f32,
    /// Current value in [0, 1].
    pub val: f32,
    /// Top-left (monitor-local) the picker is anchored to.
    pub origin: (i32, i32),
    /// Which overlay last hosted the picker — clicks elsewhere close it.
    pub overlay_idx: usize,
    /// `true` after a click → stays open. `false` for hover-opened
    /// popups, which close when the pointer leaves both the triggering
    /// swatch and the popup itself.
    pub pinned: bool,
}

/// Region of the picker a pointer click landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HsvHit {
    Hue,
    Sv,
}

impl HsvPicker {
    fn outer_rect(&self) -> (i32, i32, u32, u32) {
        (
            self.origin.0,
            self.origin.1,
            HSV_PICKER_W as u32,
            HSV_PICKER_H as u32,
        )
    }
    fn hue_rect(&self) -> (i32, i32, u32, u32) {
        (
            self.origin.0,
            self.origin.1,
            HSV_PICKER_W as u32,
            HSV_HUE_H as u32,
        )
    }
    fn sv_rect(&self) -> (i32, i32, u32, u32) {
        (
            self.origin.0,
            self.origin.1 + HSV_HUE_H + HSV_GAP_Y,
            HSV_PICKER_W as u32,
            HSV_SV_H as u32,
        )
    }
    fn foot_rect(&self) -> (i32, i32, u32, u32) {
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

    /// What region of the picker did the user click on?
    fn hit(&self, lx: i32, ly: i32) -> Option<HsvHit> {
        let inside = |r: (i32, i32, u32, u32), lx: i32, ly: i32| {
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

    /// Returns (saturation, value) for the click position inside the SV
    /// square: x → value (0 dark on the left, 1 bright on the right);
    /// y → saturation (0 white at the top, 1 fully-saturated at the
    /// bottom). Matches the Photoshop convention most users expect.
    fn sv_at(&self, lx: i32, ly: i32) -> (f32, f32) {
        let r = self.sv_rect();
        let v = ((lx - r.0).clamp(0, r.2 as i32 - 1) as f32) / (r.2 as f32);
        let s = ((ly - r.1).clamp(0, r.3 as i32 - 1) as f32) / (r.3 as f32);
        (s, v)
    }

    /// Resolved RGB at the picker's currently-selected H/S/V.
    pub fn current_rgb(&self) -> [u8; 3] {
        hsv_to_rgb(self.hue, self.sat, self.val)
    }
}

/// RGB → HSV conversion. Returns (hue ∈ [0,1), sat ∈ [0,1], val ∈ [0,1]).
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

/// HSV → RGB conversion.
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

/// Paints the full HSV picker — hue strip on top, SV square below, then
/// a 22-px footer with the current swatch and `#RRGGBB` readout.
fn draw_hsv_picker(bytes: &mut [u8], w: u32, h: u32, picker: &HsvPicker) {
    let (ox, oy, pw, ph) = picker.outer_rect();
    // Background panel + border with a small drop shadow.
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

    // Hue strip --------------------------------------------------------
    let (hx, hy, hw, hh) = picker.hue_rect();
    for ix in 0..hw as i32 {
        let hue = ix as f32 / hw as f32;
        let rgb = hsv_to_rgb(hue, 1.0, 1.0);
        for iy in 0..hh as i32 {
            paint_pixel(bytes, w, h, (hx + ix) as u32, (hy + iy) as u32, rgb);
        }
    }
    // Selected hue tick.
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

    // Saturation/value square -----------------------------------------
    let (sx, sy, sw, sh) = picker.sv_rect();
    for iy in 0..sh as i32 {
        let s = iy as f32 / sh as f32;
        for ix in 0..sw as i32 {
            let v = ix as f32 / sw as f32;
            let rgb = hsv_to_rgb(picker.hue, s, v);
            paint_pixel(bytes, w, h, (sx + ix) as u32, (sy + iy) as u32, rgb);
        }
    }
    // Selected (sat, val) marker — small circle, contrast-aware.
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

    // Footer: swatch + hex readout ------------------------------------
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

// ============================================================================
// Stroke-width slider popup
// ============================================================================

const WIDTH_POPUP_W: i32 = 220;
const WIDTH_POPUP_H: i32 = 44;
const WIDTH_MIN: f32 = 1.0;
const WIDTH_MAX: f32 = 50.0;

/// Quick-access menu opened by right-click — a small grid of square
/// colour swatches plus a row of stroke-width discs. Replaces the
/// earlier angular "radial" layout, whose pie-slice hit-tests were
/// fiddly close to the boundaries.
pub(crate) struct RadialMenu {
    /// Top-left, monitor-local.
    pub origin: (i32, i32),
    pub overlay_idx: usize,
}

const RADIAL_CELL: i32 = 26;
const RADIAL_GAP: i32 = 4;
const RADIAL_COLS: i32 = 4;
const RADIAL_PAD: i32 = 8;
/// Stroke-width presets — the row beneath the colour grid.
const RADIAL_WIDTHS: &[f32] = &[1.0, 3.0, 6.0, 12.0];

impl RadialMenu {
    /// Returns the menu's outer rect in monitor-local pixels for a
    /// palette of the given length.
    fn outer_rect(&self, palette_len: usize) -> (i32, i32, u32, u32) {
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

    fn color_cell_rect(&self, palette_len: usize, i: usize) -> (i32, i32, u32, u32) {
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

    fn width_cell_rect(&self, palette_len: usize, i: usize) -> (i32, i32, u32, u32) {
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
        for i in 0..palette_len {
            if rect_contains(self.color_cell_rect(palette_len, i), lx, ly, 0) {
                return Some(i);
            }
        }
        None
    }

    fn slot_width(&self, lx: i32, ly: i32, palette_len: usize) -> Option<usize> {
        for i in 0..RADIAL_WIDTHS.len() {
            if rect_contains(self.width_cell_rect(palette_len, i), lx, ly, 0) {
                return Some(i);
            }
        }
        None
    }

    fn contains(&self, palette_len: usize, lx: i32, ly: i32) -> bool {
        rect_contains(self.outer_rect(palette_len), lx, ly, 4)
    }
}

/// Gizmo drag state — which handle is being pulled.
#[derive(Clone, Debug)]
pub(crate) enum TransformDrag {
    /// Uniform scale from a corner; `anchor` is the opposite corner held
    /// fixed, `start_dist` is its initial radial distance at drag-start.
    Scale {
        anchor: FPoint,
        start_dist: f32,
        original: Box<crate::shape::Shape>,
    },
    /// Rotation about the bounds centre; `start_angle` is the angle
    /// (radians) from the centre to the pointer at drag-start.
    Rotate {
        center: FPoint,
        start_angle: f32,
        original: Box<crate::shape::Shape>,
    },
}

pub(crate) struct WidthPopup {
    /// Top-left, monitor-local.
    pub origin: (i32, i32),
    pub overlay_idx: usize,
    /// True while the user holds the LMB after clicking inside the slider.
    pub dragging: bool,
    /// `true` after a click → stays open until clicked away. `false` when
    /// opened by hover → closes as soon as the pointer leaves both the
    /// triggering chip and the popup's hit-area.
    pub pinned: bool,
}

impl WidthPopup {
    fn outer_rect(&self) -> (i32, i32, u32, u32) {
        (
            self.origin.0,
            self.origin.1,
            WIDTH_POPUP_W as u32,
            WIDTH_POPUP_H as u32,
        )
    }
    fn track_rect(&self) -> (i32, i32, u32, u32) {
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

// ============================================================================
// Snap-step popup
// ============================================================================
//
// Standalone widget — *not* a reskin of `WidthPopup`. Lives in its own
// `State.snap_popup` slot with its own renderer, hit-test and value
// range. The previous shared-implementation approach kept biting us:
// the kind enum and merged geometry made every fix touch both popups,
// and the dotted preview kept losing pixels whenever the stroke
// slider grew or shrank. Splitting them removes the coupling entirely.

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
    fn outer_rect(&self) -> (i32, i32, u32, u32) {
        (
            self.origin.0,
            self.origin.1,
            SNAP_POPUP_W as u32,
            SNAP_POPUP_H_INNER as u32,
        )
    }
    fn track_rect(&self) -> (i32, i32, u32, u32) {
        (
            self.origin.0 + 14,
            self.origin.1 + 32,
            (SNAP_POPUP_W - 28) as u32,
            6,
        )
    }
    fn preview_rect(&self) -> (i32, i32, u32, u32) {
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
        // Generous slack so the user doesn't have to nail the 6-px track.
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

    // Centred title.
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

    // Slider track.
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

    // Dotted preview — drawn at the *real* step, sized to the popup's
    // dedicated band so we always fit at least one row even at the
    // maximum step. 3×3 dots so they're visible on dark backgrounds.
    let preview = p.preview_rect();
    fill_rect_bytes(bytes, w, h, preview, [10, 16, 20]);
    outline_rect(bytes, w, h, preview, [60, 90, 100]);
    let step = value.round().max(SNAP_STEP_MIN) as i32;
    let dot = 3i32;
    // First-row vertical centring keeps a dot visible even at step
    // values bigger than the preview's height (the preview is ~48 px
    // tall, so steps ≥ 50 only show one row).
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
    /// The persistent stroke width — what the chip shows on the toolbar.
    fn active_tool_width(&self) -> f32 {
        self.current_width
    }

    /// Update the persistent stroke width *and* push it onto the active
    /// tool so the next stroke uses it immediately.
    fn update_current_width(&mut self, w: f32) {
        self.current_width = w.clamp(WIDTH_MIN, WIDTH_MAX);
        set_active_tool_width(&mut self.canvas.active_tool, self.current_width);
    }

    fn step_active_tool_width(&mut self, delta: f32) {
        let cur = self.current_width;
        self.update_current_width(cur + delta);
    }

    /// `true` if the pointer is anywhere inside the picker's outer rect
    /// (including the panel padding) on the overlay that owns the
    /// picker. The click handler uses this to consume *all* clicks
    /// inside the popup, not just the ones landing on the interactive
    /// hue/sv regions — otherwise a click on the panel border would
    /// fall through to the canvas and start a stroke.
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

    /// True if the pointer is currently hovering over the width-button on
    /// the toolbar. Used to scope wheel-scroll adjustments to that one
    /// button instead of having scroll fire from anywhere.
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

/// Paint the quick-access menu — a grid of colour swatches with a row
/// of stroke-width discs underneath. The rectangular cells make
/// pointer hit-tests behave predictably.
///
/// `hover_color` / `hover_width` mark the cell the pointer is currently
/// over so it can be rendered with a highlight outline.
fn draw_radial_menu(
    bytes: &mut [u8],
    w: u32,
    h: u32,
    menu: &RadialMenu,
    palette: &[crate::color::Color],
    hover_color: Option<usize>,
    hover_width: Option<usize>,
) {
    let palette_len = palette.len();
    let outer = menu.outer_rect(palette_len);
    fill_rect_bytes(bytes, w, h, outer, [22, 22, 24]);
    outline_rect(bytes, w, h, outer, [80, 80, 84]);

    // Colour cells.
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

    // Width cells — small disc centred in each square.
    for (i, w_val) in RADIAL_WIDTHS.iter().enumerate() {
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

    // Bottom readout — only when something is hovered.
    let readout = match (hover_color, hover_width) {
        (Some(i), _) if i < palette.len() => {
            let c = palette[i];
            Some(format!("#{:02X}{:02X}{:02X}", c.0[0], c.0[1], c.0[2]))
        }
        (_, Some(j)) if j < RADIAL_WIDTHS.len() => {
            Some(format!("W {}px", RADIAL_WIDTHS[j].round() as i32))
        }
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

/// Paint a faint grid of dots clipped to `clip` (monitor-local pixels).
/// Used to indicate where the snapper is going to pull the cursor —
/// only drawn inside the active editing region.
///
/// `mon_origin` is the monitor's *global* (x, y) so the rendered grid
/// can align to the same global step grid that `snap_point` snaps to.
/// Without that alignment a dot drawn at local (0, 0) lives at global
/// `mon_origin`, which is rarely a snap target.
fn draw_snap_grid_clip(
    bytes: &mut [u8],
    w: u32,
    h: u32,
    step: f32,
    clip: (i32, i32, u32, u32),
    mon_origin: (i32, i32),
) {
    let step = step.max(2.0) as i32;
    let col = [180, 180, 200];
    let (cx, cy, cw, ch) = clip;
    let x0 = cx.max(0);
    let y0 = cy.max(0);
    let x1 = (cx + cw as i32).min(w as i32);
    let y1 = (cy + ch as i32).min(h as i32);
    // First grid line ≥ x0 at a multiple of `step` in *global* coords.
    // Local x is global x − mon_origin.x; we want
    //   (local + mon.x) % step == 0
    // → local ≡ (-mon.x) mod step
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

// ============================================================================
// Magnifier HUD
// ============================================================================

const MAGNIFIER_RADIUS: i32 = 64;
const MAGNIFIER_ZOOM: i32 = 4;

fn draw_magnifier(bytes: &mut [u8], w: u32, h: u32, idx: usize, state: &State) {
    // Render the magnifier HUD whenever the user explicitly toggled it
    // *or* the pipette is armed — the pipette ergonomics improve a lot
    // with a zoomed view of the pixel about to be sampled, and the hex
    // readout doubles as the colour the next click will pick.
    if !state.magnifier_on && !state.pipette_pending {
        return;
    }
    let mon = state.overlays[idx].monitor.bounds();
    let cx = state.pointer_pos_global.x as i32 - mon.x();
    let cy = state.pointer_pos_global.y as i32 - mon.y();
    // Offset the magnifier so it doesn't cover the pixel being inspected.
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
            // Source pixel (zoomed).
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
    // Ring around the magnifier.
    for theta in 0..360 {
        let a = (theta as f32).to_radians();
        let mx = ox + (MAGNIFIER_RADIUS as f32 * a.cos()).round() as i32;
        let my = oy + (MAGNIFIER_RADIUS as f32 * a.sin()).round() as i32;
        paint_pixel(bytes, w, h, mx as u32, my as u32, [255, 255, 255]);
    }
    // Crosshair on the centre pixel being inspected.
    for d in -2..=2 {
        paint_pixel(bytes, w, h, (ox + d) as u32, oy as u32, [255, 255, 255]);
        paint_pixel(bytes, w, h, ox as u32, (oy + d) as u32, [255, 255, 255]);
    }

    // Hex readout — sample the central pixel and print "#RRGGBB" just
    // below the ring so the user can read off the colour they're hovering.
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
        // Backdrop so the hex is readable on any wallpaper.
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

// ============================================================================
// Snapping helpers
// ============================================================================

const SNAP_TOL: f32 = 8.0;

/// Snap `p` to the nearest grid intersection at `step` pixels. Used by
/// the transform gizmo and by `snap_point` so every snap path agrees on
/// where the grid is.
fn snap_point_step(p: FPoint, step: f32) -> FPoint {
    let step = step.max(2.0);
    FPoint::new((p.x / step).round() * step, (p.y / step).round() * step)
}

/// Snap `p` to the closest grid intersection (at `step` px) or to an
/// existing shape vertex, whichever is closer. Returns the original
/// point when nothing's within tolerance.
fn snap_point(canvas: &Canvas, p: FPoint, step: f32) -> FPoint {
    let mut best = snap_point_step(p, step);
    let mut best_d2 = (best.x - p.x).powi(2) + (best.y - p.y).powi(2);
    // Snap to shape vertices when they're closer than the grid snap.
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
    /// Sample the pixel under the cursor from the eager-captured
    /// background and apply it as the active stroke colour.
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

    /// Apply a colour pick (from the picker, pipette or radial menu).
    /// Without Shift the stroke colour updates; if fill mode is on, the
    /// fill colour also tracks the stroke so the user only has to pick
    /// once. Shift+pick targets the fill colour exclusively.
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

    /// Hit-test the HSV picker against the active pointer position.
    /// Returns the kind of region that was hit plus the picker-local
    /// (lx, ly) coordinates so the caller can resolve a fresh hue or
    /// (sat, val) value.
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

    /// Apply a picker click at `(lx, ly)`: hue-strip clicks update the
    /// hue, SV-square clicks update saturation/value. Either way the
    /// resolved RGB becomes the new stroke (or fill, with Shift).
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

/// Free fn so the click handler can call it without owning the borrow.
fn sample_pipette(state: &mut State) {
    state.pipette_apply();
}
