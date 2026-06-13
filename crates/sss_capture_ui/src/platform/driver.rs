//! Top-level winit-based driver, with optional egui editor toolbar.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use sss_capture::Image as CapImage;
use sss_capture::Monitor;
use sss_core::ocr::TextBox;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId as WinitWindowId};

use crate::canvas::{Canvas, CanvasEvent};
use crate::geometry::FPoint;
use crate::mode::SelectorMode;
use crate::selector::{Outcome, PostAction, Selection, Selector, SelectorError};
use crate::trigger::CaptureTrigger;

/// Entry point invoked by `Selector::run`.
pub fn run(sel: Selector) -> Result<Selection, SelectorError> {
    let Selector { config, capturer } = sel;
    // Eager mode captures up front; failure is non-fatal so the user can still
    // pick a region and the capture is retried on confirm.
    let initial = if matches!(config.trigger, CaptureTrigger::Eager) {
        match capturer.capture_all_with(config.capture_opts) {
            Ok(img) => Some(img),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "initial eager capture failed; opening the selector \
                     with no background (capture will retry on confirm)",
                );
                tracing::error!(
                    "sss_capture_ui: initial capture failed ({e}); the GUI \
                     will open without a background — the capture will be \
                     attempted again when you confirm a region."
                );
                None
            }
        }
    } else {
        None
    };

    // Kick off OCR as soon as we have the eager capture — the user's region
    // selection runs in parallel with model inference. If either the
    // pipeline is unset (OCR disabled) or the eager capture failed, skip.
    let ocr_rx: Option<Receiver<Vec<TextBox>>> = match (&initial, &config.ocr_pipeline) {
        (Some(img), Some(pipeline)) => {
            tracing::debug!("dispatching eager frame to OCR pipeline");
            Some(pipeline(img.as_rgba().clone()))
        }
        _ => None,
    };

    let monitors = capturer.monitors().map_err(SelectorError::Capture)?;

    let event_loop =
        EventLoop::new().map_err(|e| SelectorError::Backend(format!("winit event loop: {e}")))?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let save_path_hint = config.save_path_hint.clone();
    let initial_mode = match config.mode {
        SelectorMode::AnyOf => SelectorMode::Area,
        m => m,
    };
    // `EventLoop::run_app` moves and drops `app`, so the canvas / outcome /
    // action are flushed into this shared handle right before each
    // `event_loop.exit()`. `Rc<RefCell<…>>` is fine — the event loop is
    // single-threaded.
    let result: Rc<RefCell<AppResult>> = Rc::new(RefCell::new(AppResult {
        outcome: None,
        canvas: Canvas::default(),
        action: PostAction {
            copy: false,
            save: false,
            save_path_hint: save_path_hint.clone(),
            copy_text: None,
        },
    }));
    #[cfg(feature = "editor")]
    let current_color = config.palette.color_palette.first().copied().unwrap_or(
        crate::color::Color::RED,
    );
    #[cfg(feature = "editor")]
    let current_width = config.ui.default_stroke_width.max(0.5);
    #[cfg(feature = "editor")]
    let snap_step_init = config.ui.snap_step.max(2.0);
    #[cfg(feature = "editor")]
    let initial_fill = config.ui.default_fill;
    let initial_area = config.initial_area;
    let mut canvas = Canvas::default();
    if let Some(rect) = initial_area {
        canvas.set_region(Some(rect));
    }
    let app = App {
        config,
        capturer,
        monitors,
        initial,
        ocr_rx,
        windows: Vec::new(),
        canvas,
        active_window: None,
        last_cursor: FPoint::default(),
        outcome: None,
        action: PostAction {
            copy: false,
            save: false,
            save_path_hint,
            copy_text: None,
        },
        mods: ModState::default(),
        #[cfg(feature = "editor")]
        gpu: None,
        runtime_mode: initial_mode,
        result: result.clone(),
        #[cfg(feature = "editor")]
        current_color,
        #[cfg(feature = "editor")]
        current_width,
        #[cfg(feature = "editor")]
        current_fill: initial_fill,
        #[cfg(feature = "editor")]
        radial: None,
        #[cfg(feature = "editor")]
        pipette_pending: false,
        #[cfg(feature = "editor")]
        snap_on: false,
        #[cfg(feature = "editor")]
        snap_step: snap_step_init,
        #[cfg(feature = "editor")]
        magnifier_on: false,
        #[cfg(feature = "editor")]
        width_popup: None,
        #[cfg(feature = "editor")]
        snap_popup: None,
        #[cfg(feature = "editor")]
        color_popup: None,
        #[cfg(feature = "editor")]
        gizmo_drag: None,
        #[cfg(feature = "editor")]
        chrome_rects: std::collections::HashMap::new(),
        #[cfg(feature = "editor")]
        canvas_version: 0,
    };

    event_loop
        .run_app(app)
        .map_err(|e| SelectorError::Backend(format!("event loop: {e}")))?;

    let AppResult {
        outcome,
        canvas,
        action,
    } = Rc::try_unwrap(result)
        .map_err(|_| SelectorError::Backend("event-loop result handle leaked".into()))?
        .into_inner();
    Ok(Selection {
        outcome: outcome.unwrap_or(Outcome::Cancelled),
        canvas,
        action,
    })
}

#[cfg(feature = "editor")]
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

/// State extracted from the event loop after it exits.
struct AppResult {
    outcome: Option<Outcome>,
    canvas: Canvas,
    action: PostAction,
}

#[derive(Default, Clone, Copy, Debug)]
struct ModState {
    ctrl: bool,
    shift: bool,
    alt: bool,
    meta: bool,
}

struct App {
    config: crate::selector::Config,
    capturer: Arc<sss_capture::Capturer>,
    monitors: Vec<Monitor>,
    initial: Option<CapImage>,
    /// Pending OCR result. Polled in `about_to_wait`; on the first
    /// successful recv the boxes go into the canvas and the receiver
    /// is dropped so we revert to plain `ControlFlow::Wait`.
    ocr_rx: Option<Receiver<Vec<TextBox>>>,
    windows: Vec<OverlayWindow>,
    canvas: Canvas,
    active_window: Option<WinitWindowId>,
    last_cursor: FPoint,
    outcome: Option<Outcome>,
    action: PostAction,
    mods: ModState,
    #[cfg(feature = "editor")]
    gpu: Option<Arc<crate::render::gpu::Gpu>>,
    runtime_mode: SelectorMode,
    /// Output handle filled in `flush_and_exit` before the event loop drops us.
    result: Rc<RefCell<AppResult>>,
    /// Live colour applied to new strokes (mirrors `canvas.active_tool`'s
    /// colour but kept around so the toolbar swatch / radial menu have a
    /// single source of truth across tool switches).
    #[cfg(feature = "editor")]
    current_color: crate::color::Color,
    #[cfg(feature = "editor")]
    current_width: f32,
    /// Persistent fill state. `None` = outline-only mode; `Some(c)` = closed
    /// shapes are filled with `c`. Mirrors what `canvas.set_fill_color` does
    /// internally, but kept in `App` so the toolbar / radial / picker have a
    /// single source of truth across tool switches.
    #[cfg(feature = "editor")]
    current_fill: Option<crate::color::Color>,
    /// Right-click radial menu state: (overlay index, popup state, armed
    /// flag). `armed = false` on the first render frame so the click that
    /// opened the menu is not treated as an outside-click close.
    #[cfg(feature = "editor")]
    radial: Option<(usize, crate::render::ui::RadialState, bool)>,
    // Note: icon textures live per-window in `OverlayWindow.icons` —
    // `egui::TextureHandle`s are tied to a specific `egui::Context`, so
    // sharing a single cache across windows would only ever upload the
    // raster to one window's renderer.
    /// Pipette: when set, the next left-click samples the eager capture and
    /// applies the colour instead of routing the click through the canvas.
    #[cfg(feature = "editor")]
    pipette_pending: bool,
    /// Snap toggle and current grid step in pixels.
    #[cfg(feature = "editor")]
    snap_on: bool,
    #[cfg(feature = "editor")]
    snap_step: f32,
    /// Magnifier toggle.
    #[cfg(feature = "editor")]
    magnifier_on: bool,
    /// Open popup state: (overlay, window-local origin, armed). `armed`
    /// flips true on the first render frame so the opening click can't
    /// close the popup on the same frame.
    #[cfg(feature = "editor")]
    width_popup: Option<(usize, egui::Pos2, bool)>,
    #[cfg(feature = "editor")]
    snap_popup: Option<(usize, egui::Pos2, bool)>,
    #[cfg(feature = "editor")]
    color_popup: Option<(usize, egui::Pos2, bool, crate::render::ui::HsvState)>,
    /// Active gizmo drag: stores the original (pre-drag) shape and the
    /// pivot + start metric so each PointerMove recomputes the new shape
    /// directly from the original (no incremental compounding).
    #[cfg(feature = "editor")]
    gizmo_drag: Option<GizmoDrag>,
    /// Bounding rects of every visible chrome element (toolbar / popup /
    /// radial), per overlay. Stashed each render so the event handler can
    /// hit-test pointer clicks without relying on egui's stateful
    /// `wants_pointer_input`, which has proven unreliable on a layer-shell
    /// fullscreen overlay (always reports `true` after the first frame).
    #[cfg(feature = "editor")]
    chrome_rects: std::collections::HashMap<usize, Vec<egui::Rect>>,
    /// Monotonic counter bumped whenever the canvas content changes
    /// (shape commit / undo / redo / delete / clear / gizmo end). Used to
    /// invalidate the blur-source cache so the live BlurRect preview
    /// keeps up with edits.
    #[cfg(feature = "editor")]
    canvas_version: u64,
}

#[cfg(feature = "editor")]
#[derive(Clone, Debug)]
struct GizmoDrag {
    handle: crate::render::ui::GizmoHandle,
    /// Scale anchor / rotate center (always the shape's bounds centre).
    pivot: FPoint,
    /// Distance (scale) or angle in radians (rotate) at drag start.
    start_metric: f32,
    /// Snapshot of the shape at drag start; every move recomputes from
    /// this so dragging back and forth doesn't compound floating-point
    /// error.
    original: Box<crate::shape::Shape>,
}

impl App {
    /// Request a redraw on every overlay. Pointer / canvas mutations span
    /// multiple monitors (region selections, freehand strokes that cross an
    /// edge), so single-window redraw leaves stale pixels on the others.
    fn broadcast_redraw(&self) {
        for w in &self.windows {
            w.window.request_redraw();
        }
    }

    /// Pick the overlay that should host the floating toolbar. With an
    /// active region we choose the monitor with the largest intersection
    /// (so a region spanning two monitors only paints one chip); without
    /// one we fall back to the focused overlay or the first overlay.
    #[cfg(feature = "editor")]
    fn is_toolbar_host(&self, pos: usize) -> bool {
        if self.windows.is_empty() {
            return false;
        }
        // Toolbar appears only after the user has actually selected something
        // — i.e. a valid Area region exists. Monitor / Window picker modes
        // don't show the chrome (confirm via click / Enter instead).
        let Some(r) = self
            .canvas
            .region()
            .filter(|r| r.width() >= 2 && r.height() >= 2)
        else {
            return false;
        };
        let mut best: Option<(usize, u64)> = None;
        for (i, w) in self.windows.iter().enumerate() {
            if let Some(inter) = w.monitor.bounds().intersection(&r) {
                let area = inter.width() as u64 * inter.height() as u64;
                if area == 0 {
                    continue;
                }
                if best.map_or(true, |(_, a)| area > a) {
                    best = Some((i, area));
                }
            }
        }
        best.map(|(i, _)| i) == Some(pos)
    }

    /// A cheap hash of the canvas shape list — used to invalidate the
    /// blur-source cache when shapes are added, removed, or replaced.
    /// Doesn't catch in-place coord changes during a gizmo drag, but the
    /// drag's final replace_shape changes shape count parity on the next
    /// frame via undo snapshot or remains the same — we accept missing
    /// re-blur until the next mutation, since rebuilding every frame is
    /// too expensive for 4K monitors.
    #[cfg(feature = "editor")]
    fn canvas_state_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for s in self.canvas.shapes() {
            s.id.0.hash(&mut h);
            std::mem::discriminant(&s.kind).hash(&mut h);
            // Coord proxies: hash bounds so transform drag invalidates too.
            let b = s.bounds();
            b.x().hash(&mut h);
            b.y().hash(&mut h);
            b.width().hash(&mut h);
            b.height().hash(&mut h);
        }
        h.finish()
    }

    /// Pick the cursor that should be shown right now and push it to
    /// every overlay window. Called whenever pointer / canvas / chrome
    /// state changes (pointer moves, tool switches, gizmo hover).
    #[cfg(feature = "editor")]
    fn refresh_cursor(&self, current_overlay: Option<usize>) {
        use winit::cursor::{Cursor, CursorIcon};
        let on_chrome = current_overlay
            .and_then(|p| self.chrome_rects.get(&p))
            .map(|rs| {
                let win_local = current_overlay
                    .map(|p| {
                        let o = self.windows[p].monitor.bounds();
                        egui::Pos2::new(
                            self.last_cursor.x - o.x() as f32,
                            self.last_cursor.y - o.y() as f32,
                        )
                    })
                    .unwrap_or(egui::Pos2::ZERO);
                rs.iter().any(|r| r.contains(win_local))
            })
            .unwrap_or(false);
        let on_gizmo = {
            if let Some(sel) = self.canvas.selected() {
                if let Some(shape) = self.canvas.shapes().iter().find(|s| s.id == sel) {
                    crate::render::ui::hit_gizmo(shape.bounds(), self.last_cursor).is_some()
                } else {
                    false
                }
            } else {
                false
            }
        };
        let name = crate::cursor::desired_cursor_ext(
            &self.canvas,
            self.last_cursor,
            on_chrome,
            on_gizmo,
            self.runtime_mode,
        );
        let icon = match name {
            crate::cursor::CursorName::Default => CursorIcon::Default,
            crate::cursor::CursorName::Crosshair => CursorIcon::Crosshair,
            crate::cursor::CursorName::Move => CursorIcon::Move,
            crate::cursor::CursorName::NwSeResize => CursorIcon::NwseResize,
            crate::cursor::CursorName::NeSwResize => CursorIcon::NeswResize,
            crate::cursor::CursorName::NsResize => CursorIcon::NsResize,
            crate::cursor::CursorName::EwResize => CursorIcon::EwResize,
        };
        for w in &self.windows {
            w.window.set_cursor(Cursor::Icon(icon));
        }
    }

    /// Apply a colour pick globally. With `fill_only = true` (Shift +
    /// pipette / fill chip), only the fill changes; otherwise the stroke
    /// changes and fill tracks it when fill mode is on.
    #[cfg(feature = "editor")]
    fn apply_color_pick(&mut self, c: crate::color::Color, fill_only: bool) {
        if fill_only {
            self.current_fill = Some(c);
            self.canvas.set_fill_color(Some(c));
            return;
        }
        self.current_color = c;
        if self.current_fill.is_some() {
            self.current_fill = Some(c);
        }
        self.push_current_to_tool();
    }

    /// Apply a width pick globally.
    #[cfg(feature = "editor")]
    fn apply_width_pick(&mut self, w: f32) {
        self.current_width = w.max(0.5);
        self.push_current_to_tool();
    }

    /// Re-seat the active tool's brush colour / stroke width / fill from
    /// the persistent `current_*` fields. Call this after every action
    /// that changes the global state (tool switch, colour pick, width
    /// change, pipette sample), so the next stroke uses the user's
    /// current choices regardless of which tool is selected.
    #[cfg(feature = "editor")]
    fn push_current_to_tool(&mut self) {
        crate::icons::set_active_tool_width(&mut self.canvas.active_tool, self.current_width);
        set_active_tool_color(&mut self.canvas.active_tool, self.current_color);
        self.canvas.set_fill_color(self.current_fill);
    }

    fn flush_and_exit(&mut self, event_loop: &dyn ActiveEventLoop) {
        let mut r = self.result.borrow_mut();
        r.outcome = self.outcome.take();
        r.canvas = std::mem::take(&mut self.canvas);
        r.action = PostAction {
            copy: self.action.copy,
            save: self.action.save,
            save_path_hint: self.action.save_path_hint.take(),
            copy_text: self.action.copy_text.take(),
        };
        drop(r);
        event_loop.exit();
    }

    /// Pump the OCR result channel.
    ///
    /// While the worker is in flight we ask winit to wake us every
    /// 200 ms; the moment a `Vec<TextBox>` lands we copy it into the
    /// canvas, drop the receiver (so the next call is a no-op) and let
    /// the loop fall back to `ControlFlow::Wait`.
    fn poll_ocr(&mut self, event_loop: &dyn ActiveEventLoop) {
        let Some(rx) = &self.ocr_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(boxes) => {
                tracing::info!(count = boxes.len(), "OCR result received");
                self.canvas.set_text_boxes(boxes);
                self.ocr_rx = None;
                event_loop.set_control_flow(ControlFlow::Wait);
                self.broadcast_redraw();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                let until =
                    std::time::Instant::now() + std::time::Duration::from_millis(200);
                event_loop.set_control_flow(ControlFlow::WaitUntil(until));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                tracing::warn!("OCR worker disconnected without a result");
                self.ocr_rx = None;
                event_loop.set_control_flow(ControlFlow::Wait);
            }
        }
    }
}

struct OverlayWindow {
    window: Arc<dyn Window>,
    monitor: Monitor,
    #[cfg(feature = "editor")]
    gpu: Option<crate::render::gpu::WindowGpu>,
    /// Eager-captured monitor slice uploaded as an egui texture, painted as
    /// background under the canvas. Allocated lazily on first render.
    #[cfg(feature = "editor")]
    background: Option<egui::TextureHandle>,
    /// CPU-blurred "everything below the blur layer" copy. Rebuilt
    /// lazily whenever the canvas version changes so the live BlurRect
    /// preview honours strokes / shapes painted on top of the desktop.
    /// Stored as (canvas_version, texture). When the bg has nothing on
    /// top, this is just a blurred bg slice.
    #[cfg(feature = "editor")]
    blur_source: Option<(u64, egui::TextureHandle)>,
    /// Per-window icon cache. `TextureHandle`s are bound to a single
    /// `egui::Context`, so each window uploads its own rasters.
    #[cfg(feature = "editor")]
    icons: crate::render::ui::IconCache,
}

impl ApplicationHandler for App {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        tracing::info!("App::can_create_surfaces — creating overlay windows");
        if !self.windows.is_empty() {
            tracing::debug!("resume after suspend; reusing existing windows");
            return;
        }
        let winit_monitors: Vec<_> = event_loop.available_monitors().collect();
        // On Wayland we take the wlr-layer-shell path (one layer surface per
        // wl_output anchored to all four edges), which gives us a real
        // fullscreen overlay above panels without depending on xdg-shell
        // fullscreen quirks. On X11 / other backends we fall back to a
        // borderless fullscreen `xdg_toplevel`.
        #[cfg(target_os = "linux")]
        let wayland = {
            use winit::platform::wayland::ActiveEventLoopExtWayland as _;
            event_loop.is_wayland()
        };
        #[cfg(not(target_os = "linux"))]
        let wayland = false;
        tracing::info!(
            wayland,
            "winit reports {} available monitor(s); sss_capture reports {}",
            winit_monitors.len(),
            self.monitors.len()
        );
        for (i, monitor) in self.monitors.iter().enumerate() {
            let target = winit_monitors.iter().find(|m| {
                m.position().is_some_and(|pos| {
                    pos.x == monitor.bounds().x() && pos.y == monitor.bounds().y()
                })
            });
            // `Borderless(None)` lets the compositor pick the current output
            // when winit can't enumerate (some Wayland setups).
            let fullscreen = (!wayland)
                .then(|| Some(winit::monitor::Fullscreen::Borderless(target.cloned())))
                .flatten();
            tracing::info!(
                monitor = %monitor.name(),
                index = i,
                handle_matched = target.is_some(),
                bounds = %monitor.bounds(),
                wayland,
                "creating overlay window",
            );
            let mut attrs = winit::window::WindowAttributes::default()
                .with_title("sss_capture_ui overlay")
                .with_decorations(false)
                .with_resizable(false)
                .with_visible(true)
                .with_active(true)
                // Without an explicit surface_size winit-Wayland can open a 0x0
                // window that the compositor then hides.
                .with_surface_size(winit::dpi::PhysicalSize::new(
                    monitor.bounds().width().max(640),
                    monitor.bounds().height().max(480),
                ))
                .with_transparent(matches!(self.config.trigger, CaptureTrigger::Lazy { .. }))
                .with_fullscreen(fullscreen);

            #[cfg(target_os = "linux")]
            if wayland {
                use winit::platform::wayland::{
                    Anchor, KeyboardInteractivity, Layer, WindowAttributesWayland,
                };
                let mut wl_attrs = WindowAttributesWayland::default()
                    .with_name("sss-capture-ui", "")
                    .with_namespace("sss-capture-ui")
                    .with_layer_shell()
                    .with_layer(Layer::Overlay)
                    .with_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT)
                    // `-1` lets the surface paint over panels and other
                    // layer-shell clients instead of being pushed aside by
                    // their exclusive zones.
                    .with_exclusive_zone(-1)
                    .with_keyboard_interactivity(KeyboardInteractivity::OnDemand);
                if let Some(handle) = target {
                    wl_attrs = wl_attrs.with_output(handle.native_id());
                }
                attrs = attrs.with_platform_attributes(Box::new(wl_attrs));
            }

            match event_loop.create_window(attrs) {
                Ok(window) => {
                    let window: Arc<dyn Window> = Arc::from(window);
                    let id = window.id();
                    window.request_redraw();
                    tracing::info!(?id, "overlay window created and redraw requested");
                    let overlay = OverlayWindow {
                        window,
                        monitor: monitor.clone(),
                        #[cfg(feature = "editor")]
                        gpu: None,
                        #[cfg(feature = "editor")]
                        background: None,
                        #[cfg(feature = "editor")]
                        blur_source: None,
                        #[cfg(feature = "editor")]
                        icons: crate::render::ui::IconCache::default(),
                    };
                    self.windows.push(overlay);
                }
                Err(e) => {
                    tracing::error!(
                        "sss_capture_ui: failed to create overlay window for {monitor}: {e}"
                    );
                    tracing::error!(error = %e, "failed to create overlay window for {monitor}");
                }
            }
        }
        tracing::info!("opened {} overlay window(s)", self.windows.len());

        #[cfg(feature = "editor")]
        if self.config.toolbar {
            tracing::info!("initialising wgpu device for the editor toolbar");
            self.init_gpu();
            tracing::info!(
                "wgpu init complete (per-window state ready for {} window(s))",
                self.windows.iter().filter(|w| w.gpu.is_some()).count()
            );
        }
    }

    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.poll_ocr(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WinitWindowId,
        event: WindowEvent,
    ) {
        let (origin, _monitor) = match self.windows.iter().find(|w| w.window.id() == id) {
            Some(w) => (
                (w.monitor.bounds().x(), w.monitor.bounds().y()),
                w.monitor.clone(),
            ),
            None => return,
        };
        // Only pointer events update which overlay holds the cursor.
        // Updating from every event (including RedrawRequested) made the
        // magnifier render on every monitor because each broadcast redraw
        // would set `active_window` to the rendering monitor's id.
        if matches!(
            &event,
            WindowEvent::PointerMoved { .. }
                | WindowEvent::PointerEntered { .. }
                | WindowEvent::PointerButton { .. }
        ) {
            self.active_window = Some(id);
        }

        // Feed every event to egui so widget interaction works (hover,
        // click, drag inside the toolbar / popups). We deliberately ignore
        // egui's `consumed` flag here: on a layer-shell fullscreen overlay
        // it has proven to return `true` after the first frame even for
        // clicks on the bare canvas. Instead we hit-test the pointer
        // against the chrome rects we stashed during the previous render
        // pass (see `chrome_rects`).
        let pos_local = match &event {
            WindowEvent::PointerMoved { position, .. }
            | WindowEvent::PointerButton { position, .. } => Some(egui::Pos2::new(
                position.x as f32,
                position.y as f32,
            )),
            _ => None,
        };
        #[cfg(feature = "editor")]
        {
            if let Some(win) = self.windows.iter_mut().find(|w| w.window.id() == id) {
                if let Some(wg) = win.gpu.as_mut() {
                    let resp = wg.egui_winit.on_window_event(&*win.window, &event);
                    if resp.repaint {
                        win.window.request_redraw();
                    }
                }
            }
        }
        #[cfg(feature = "editor")]
        let egui_consumed = {
            let win_pos = self.windows.iter().position(|w| w.window.id() == id);
            match (win_pos, pos_local) {
                (Some(p), Some(pt)) => self
                    .chrome_rects
                    .get(&p)
                    .map(|rects| rects.iter().any(|r| r.contains(pt)))
                    .unwrap_or(false),
                _ => false,
            }
        };
        #[cfg(not(feature = "editor"))]
        let egui_consumed = false;

        match event {
            WindowEvent::CloseRequested => {
                self.outcome = Some(Outcome::Cancelled);
                self.flush_and_exit(event_loop);
            }
            WindowEvent::ModifiersChanged(state) => {
                let m = state.state();
                self.mods.ctrl = m.control_key();
                self.mods.shift = m.shift_key();
                self.mods.alt = m.alt_key();
                self.mods.meta = m.meta_key();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                match event.logical_key.as_ref() {
                    Key::Named(NamedKey::Escape) => {
                        self.outcome = Some(Outcome::Cancelled);
                        self.flush_and_exit(event_loop);
                    }
                    Key::Named(NamedKey::Enter) => {
                        if self.canvas.is_drawing_polygon() {
                            self.canvas.commit_polygon();
                            self.broadcast_redraw();
                        } else if self.config.confirm_with_enter {
                            self.confirm(event_loop);
                        }
                    }
                    Key::Named(NamedKey::Backspace) => {
                        self.canvas.handle(CanvasEvent::TextBackspace);
                    }
                    Key::Character("z") | Key::Character("Z") if self.mods.ctrl => {
                        if self.mods.shift {
                            self.canvas.handle(CanvasEvent::Redo);
                        } else {
                            self.canvas.handle(CanvasEvent::Undo);
                        }
                    }
                    Key::Character("y") | Key::Character("Y") if self.mods.ctrl => {
                        self.canvas.handle(CanvasEvent::Redo);
                    }
                    Key::Character("c") | Key::Character("C") if self.mods.ctrl => {
                        // OCR text first: Ctrl+C with a non-empty selection
                        // copies the joined text rather than the image. The
                        // CLI side checks `copy_text` ahead of `copy` and
                        // bypasses the image pipeline when present.
                        if let Some(text) = self.canvas.selected_text() {
                            self.action.copy_text = Some(text);
                        } else {
                            self.action.copy = true;
                        }
                        self.confirm(event_loop);
                    }
                    Key::Character("s") | Key::Character("S") if self.mods.ctrl => {
                        self.action.save = true;
                        self.confirm(event_loop);
                    }
                    Key::Named(NamedKey::Delete) => {
                        self.canvas.handle(CanvasEvent::Delete);
                    }
                    Key::Character(s) => {
                        if let Some(ch) = s.chars().next() {
                            if !self.mods.ctrl && !self.mods.alt && !self.mods.meta {
                                self.canvas.handle(CanvasEvent::TextInput(ch));
                            }
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::PointerMoved { position, .. } => {
                let raw = FPoint::new(
                    position.x as f32 + origin.0 as f32,
                    position.y as f32 + origin.1 as f32,
                );
                self.last_cursor = raw;
                #[cfg(feature = "editor")]
                let p = if self.snap_on {
                    crate::render::ui::snap_point(&self.canvas, raw, self.snap_step)
                } else {
                    raw
                };
                #[cfg(not(feature = "editor"))]
                let p = raw;
                // Gizmo drag wins over canvas: scale / rotate the selection
                // Apply transform from the original snapshot — no
                // incremental drift, identical to the legacy driver.
                #[cfg(feature = "editor")]
                if let Some(g) = self.gizmo_drag.clone() {
                    if let Some(sel) = self.canvas.selected() {
                        let dx = p.x - g.pivot.x;
                        let dy = p.y - g.pivot.y;
                        let mut new_shape = (*g.original).clone();
                        match g.handle {
                            crate::render::ui::GizmoHandle::Scale => {
                                let dist = (dx * dx + dy * dy).sqrt().max(1.0);
                                let factor =
                                    (dist / g.start_metric.max(1.0)).clamp(0.05, 20.0);
                                crate::canvas::scale_shape_about(
                                    &mut new_shape,
                                    g.pivot,
                                    factor,
                                );
                            }
                            crate::render::ui::GizmoHandle::Rotate => {
                                let ang = dy.atan2(dx);
                                crate::canvas::rotate_shape_about(
                                    &mut new_shape,
                                    g.pivot,
                                    ang - g.start_metric,
                                );
                            }
                        }
                        new_shape.id = sel;
                        self.canvas.replace_shape(sel, new_shape);
                    }
                    self.broadcast_redraw();
                    return;
                }
                self.canvas.handle(CanvasEvent::PointerMove(p));
                #[cfg(feature = "editor")]
                {
                    let cur_pos = self.windows.iter().position(|w| w.window.id() == id);
                    self.refresh_cursor(cur_pos);
                }
                // Drag-selection rectangles span multiple monitors; redraw
                // every overlay so the region preview stays consistent.
                self.broadcast_redraw();
            }
            WindowEvent::PointerButton {
                state,
                button,
                position,
                ..
            } => match button.mouse_button() {
                Some(MouseButton::Left) => {
                    // Click landed on the toolbar / a popup — egui owns it.
                    if egui_consumed {
                        return;
                    }
                    // Pipette short-circuit: sample the eager capture at the
                    // pointer and apply the colour instead of routing the
                    // click to the canvas tool.
                    #[cfg(feature = "editor")]
                    if self.pipette_pending && state == ElementState::Pressed {
                        if let Some(initial) = self.initial.as_ref() {
                            let bb = sss_capture::Rect::bounding(
                                &self
                                    .monitors
                                    .iter()
                                    .map(|m| m.bounds())
                                    .collect::<Vec<_>>(),
                            )
                            .unwrap_or_default();
                            if let Some(c) = crate::render::ui::sample_capture(
                                initial,
                                (bb.x(), bb.y()),
                                (
                                    self.last_cursor.x as i32,
                                    self.last_cursor.y as i32,
                                ),
                            ) {
                                let fill_only = self.mods.shift;
                                self.apply_color_pick(c, fill_only);
                            }
                        }
                        self.pipette_pending = false;
                        self.broadcast_redraw();
                        return;
                    }
                    let pt = {
                        #[cfg(feature = "editor")]
                        {
                            if self.snap_on {
                                crate::render::ui::snap_point(
                                    &self.canvas,
                                    self.last_cursor,
                                    self.snap_step,
                                )
                            } else {
                                self.last_cursor
                            }
                        }
                        #[cfg(not(feature = "editor"))]
                        {
                            self.last_cursor
                        }
                    };
                    match state {
                        ElementState::Pressed => {
                            // Gizmo hit-test: if a shape is selected and the
                            // pointer hits a transform handle, start a gizmo
                            // drag instead of routing the click to the
                            // canvas (which would deselect or start a move).
                            #[cfg(feature = "editor")]
                            {
                                if let Some(sel) = self.canvas.selected() {
                                    if let Some(shape) = self
                                        .canvas
                                        .shapes()
                                        .iter()
                                        .find(|s| s.id == sel)
                                        .cloned()
                                    {
                                        let bounds = shape.bounds();
                                        if let Some(h) = crate::render::ui::hit_gizmo(
                                            bounds, self.last_cursor,
                                        ) {
                                            let cx = bounds.x() as f32
                                                + bounds.width() as f32 / 2.0;
                                            let cy = bounds.y() as f32
                                                + bounds.height() as f32 / 2.0;
                                            let dx = pt.x - cx;
                                            let dy = pt.y - cy;
                                            let metric = match h {
                                                crate::render::ui::GizmoHandle::Scale => {
                                                    (dx * dx + dy * dy).sqrt().max(1.0)
                                                }
                                                crate::render::ui::GizmoHandle::Rotate => {
                                                    dy.atan2(dx)
                                                }
                                            };
                                            self.gizmo_drag = Some(GizmoDrag {
                                                handle: h,
                                                pivot: FPoint::new(cx, cy),
                                                start_metric: metric,
                                                original: Box::new(shape),
                                            });
                                            self.broadcast_redraw();
                                            return;
                                        }
                                    }
                                }
                            }
                            // OCR selection: when the Pointer tool is active
                            // and the click lands inside a recognised text
                            // polygon, toggle that box in the selection set
                            // and swallow the event so PointerDown doesn't
                            // start a region drag.
                            if matches!(
                                self.canvas.active_tool,
                                crate::tool::Tool::Pointer
                            ) && self.canvas.has_ocr()
                            {
                                if let Some(idx) = self.canvas.ocr_hit(pt.x, pt.y) {
                                    self.canvas.toggle_text_box_selection(idx);
                                    self.broadcast_redraw();
                                    return;
                                }
                            }
                            self.canvas.handle(CanvasEvent::PointerDown(pt));
                        }
                        ElementState::Released => {
                            #[cfg(feature = "editor")]
                            if self.gizmo_drag.is_some() {
                                self.gizmo_drag = None;
                                self.broadcast_redraw();
                                return;
                            }
                            self.canvas.handle(CanvasEvent::PointerUp(pt));
                        }
                    }
                    self.broadcast_redraw();
                }
                Some(MouseButton::Right) if state == ElementState::Pressed => {
                    if egui_consumed {
                        return;
                    }
                    #[cfg(feature = "editor")]
                    {
                        // Polygon-in-progress: right-click commits.
                        if self.canvas.is_drawing_polygon() {
                            self.canvas.commit_polygon();
                            self.broadcast_redraw();
                            return;
                        }
                        if self.radial.is_some() {
                            self.radial = None;
                        } else if let Some(idx) =
                            self.windows.iter().position(|w| w.window.id() == id)
                        {
                            self.radial = Some((
                                idx,
                                crate::render::ui::RadialState {
                                    origin: egui::Pos2::new(
                                        position.x as f32,
                                        position.y as f32,
                                    ),
                                },
                                false,
                            ));
                        }
                        self.broadcast_redraw();
                    }
                    #[cfg(not(feature = "editor"))]
                    let _ = position;
                }
                _ => {}
            },
            WindowEvent::RedrawRequested => {
                tracing::trace!(?id, "redraw requested");
                #[cfg(feature = "editor")]
                self.render_window(id, event_loop);
            }
            WindowEvent::Occluded(occluded) => {
                tracing::debug!(?id, occluded, "window occlusion changed");
            }
            WindowEvent::Focused(focused) => {
                tracing::debug!(?id, focused, "window focus changed");
                if focused {
                    if let Some(win) = self.windows.iter().find(|w| w.window.id() == id) {
                        win.window.request_redraw();
                    }
                }
            }
            WindowEvent::SurfaceResized(new_size) => {
                #[cfg(feature = "editor")]
                if let (Some(gpu), Some(win)) = (
                    self.gpu.clone(),
                    self.windows.iter_mut().find(|w| w.window.id() == id),
                ) {
                    if let Some(wg) = win.gpu.as_mut() {
                        wg.resize(&gpu, (new_size.width.max(1), new_size.height.max(1)));
                    }
                }
            }
            _ => {}
        }
    }
}

impl App {
    fn confirm(&mut self, event_loop: &dyn ActiveEventLoop) {
        let region = self.canvas.region();
        let outcome = match self.runtime_mode {
            SelectorMode::Monitor => {
                let p =
                    sss_capture::Point::new(self.last_cursor.x as i32, self.last_cursor.y as i32);
                if let Ok(m) = self.capturer.monitor_at(p) {
                    let image = self.capture_region(m.bounds());
                    Outcome::Monitor {
                        monitor: m.id(),
                        rect: m.bounds(),
                        image,
                    }
                } else {
                    Outcome::Cancelled
                }
            }
            SelectorMode::Window => {
                let cursor_point =
                    sss_capture::Point::new(self.last_cursor.x as i32, self.last_cursor.y as i32);
                let win = self
                    .capturer
                    .windows()
                    .ok()
                    .and_then(|ws| ws.into_iter().find(|w| w.bounds().contains(cursor_point)));
                if let Some(w) = win {
                    let image = self.capture_region(w.bounds());
                    Outcome::Window {
                        window: w.id(),
                        rect: w.bounds(),
                        image,
                    }
                } else {
                    Outcome::Cancelled
                }
            }
            SelectorMode::Area | SelectorMode::AnyOf => match region {
                Some(r) if r.width() >= 2 && r.height() >= 2 => {
                    let image = self.capture_region(r);
                    Outcome::Region { rect: r, image }
                }
                _ => Outcome::Cancelled,
            },
        };
        self.outcome = Some(outcome);
        self.flush_and_exit(event_loop);
    }

    /// Materialise the captured image for `rect`.
    fn capture_region(&self, rect: sss_capture::Rect) -> Option<CapImage> {
        let raw = match self.initial.clone() {
            Some(img) => {
                let monitors_bb = sss_capture::Rect::bounding(
                    &self.monitors.iter().map(|m| m.bounds()).collect::<Vec<_>>(),
                )
                .unwrap_or_default();
                let local_x = (rect.x() - monitors_bb.x()).max(0) as u32;
                let local_y = (rect.y() - monitors_bb.y()).max(0) as u32;
                let cropped = image::imageops::crop_imm(
                    img.as_rgba(),
                    local_x,
                    local_y,
                    rect.width(),
                    rect.height(),
                )
                .to_image();
                Some(cropped)
            }
            None => self
                .capturer
                .capture_region(rect)
                .ok()
                .map(|i| i.into_rgba()),
        };
        let mut buf = raw?;
        crate::render::composite::flatten(&mut buf, &self.canvas, (rect.x(), rect.y()));
        Some(CapImage::new(buf))
    }
}

#[cfg(feature = "editor")]
impl App {
    fn init_gpu(&mut self) {
        // wgpu 22 keeps an internal reference to the surface used to create
        // the adapter for as long as the adapter lives, so the first window's
        // surface must outlive everything else.
        if self.windows.is_empty() {
            tracing::warn!("init_gpu: no overlay windows; skipping");
            return;
        }
        tracing::info!("init_gpu: creating wgpu instance");
        let instance = crate::render::gpu::Gpu::new_instance();

        tracing::info!("init_gpu: creating surface for the first window");
        let first_window = self.windows[0].window.clone();
        let first_surface = match instance.create_surface(first_window.clone()) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("sss_capture_ui: wgpu surface creation failed: {e}");
                tracing::error!(error = %e, "wgpu: surface creation failed; editor disabled");
                return;
            }
        };

        tracing::info!("init_gpu: probing adapter / device");
        let gpu = match crate::render::gpu::Gpu::new_with_surface(instance, &first_surface) {
            Ok(g) => {
                tracing::info!(
                    adapter = ?g.adapter.get_info().name,
                    backend = ?g.adapter.get_info().backend,
                    "wgpu adapter selected",
                );
                Arc::new(g)
            }
            Err(e) => {
                tracing::error!("sss_capture_ui: wgpu init failed: {e}");
                tracing::error!(error = %e, "wgpu init failed; editor disabled");
                return;
            }
        };

        match crate::render::gpu::WindowGpu::from_surface(first_window, first_surface, &gpu) {
            Ok(state) => {
                tracing::info!(
                    size = ?state.size,
                    format = ?state.surface_format,
                    "wgpu per-window state ready (window 0; reused probe surface)",
                );
                self.windows[0].gpu = Some(state);
            }
            Err(e) => {
                tracing::error!("sss_capture_ui: window-0 wgpu init failed: {e}");
                tracing::error!(error = %e, "wgpu: window-0 init failed");
                return;
            }
        }

        for (idx, win) in self.windows.iter_mut().enumerate().skip(1) {
            match crate::render::gpu::WindowGpu::new(win.window.clone(), &gpu) {
                Ok(state) => {
                    tracing::info!(
                        size = ?state.size,
                        format = ?state.surface_format,
                        window = idx,
                        "wgpu per-window state ready",
                    );
                    win.gpu = Some(state);
                }
                Err(e) => {
                    tracing::error!(
                        "sss_capture_ui: per-window wgpu init failed (window {idx}): {e}"
                    );
                    tracing::warn!(error = %e, window = idx, "wgpu: per-window init failed");
                }
            }
        }
        self.gpu = Some(gpu);
    }

    /// Crop the eager-capture image to `monitor[pos]`'s slice and upload it
    /// as a fresh egui texture. Returns `None` if no capture is available
    /// (lazy trigger / capture failed) or the slice is empty.
    fn build_monitor_background(
        &self,
        pos: usize,
        ctx: &egui::Context,
    ) -> Option<egui::TextureHandle> {
        let initial = self.initial.as_ref()?;
        let bounds = self.windows[pos].monitor.bounds();
        if bounds.width() == 0 || bounds.height() == 0 {
            return None;
        }
        let monitors_bb = sss_capture::Rect::bounding(
            &self.monitors.iter().map(|m| m.bounds()).collect::<Vec<_>>(),
        )
        .unwrap_or_default();
        let local_x = (bounds.x() - monitors_bb.x()).max(0) as u32;
        let local_y = (bounds.y() - monitors_bb.y()).max(0) as u32;
        let cropped = image::imageops::crop_imm(
            initial.as_rgba(),
            local_x,
            local_y,
            bounds.width(),
            bounds.height(),
        )
        .to_image();
        let (w, h) = (cropped.width() as usize, cropped.height() as usize);
        let pixels = egui::ColorImage::from_rgba_unmultiplied([w, h], cropped.as_raw());
        Some(ctx.load_texture(
            format!("sss_capture_ui::bg::{pos}"),
            pixels,
            egui::TextureOptions::LINEAR,
        ))
    }

    /// Build the "everything below the blur layer" texture for this
    /// monitor — bg slice + every non-`BlurRect` shape, then a strong
    /// Gaussian blur. Re-runs when the canvas mutates; expensive enough
    /// (CPU, ~500ms on 4K) that we cache via `OverlayWindow.blur_source`
    /// and only rebuild when `canvas_version` changes.
    fn build_blur_source(
        &self,
        pos: usize,
        ctx: &egui::Context,
    ) -> Option<egui::TextureHandle> {
        let initial = self.initial.as_ref()?;
        let bounds = self.windows[pos].monitor.bounds();
        if bounds.width() == 0 || bounds.height() == 0 {
            return None;
        }
        let monitors_bb = sss_capture::Rect::bounding(
            &self.monitors.iter().map(|m| m.bounds()).collect::<Vec<_>>(),
        )
        .unwrap_or_default();
        let local_x = (bounds.x() - monitors_bb.x()).max(0) as u32;
        let local_y = (bounds.y() - monitors_bb.y()).max(0) as u32;
        let mut cropped = image::imageops::crop_imm(
            initial.as_rgba(),
            local_x,
            local_y,
            bounds.width(),
            bounds.height(),
        )
        .to_image();
        // Composite only the shapes that sit BELOW the first BlurRect in
        // z-order onto the cropped bg. Shapes drawn after the blur
        // (visually on top) must not contribute to the blurred sample —
        // otherwise they'd ghost as a "neon halo" around their actual
        // position when seen through the blur rect.
        crate::render::composite::flatten_below_first_blur(
            &mut cropped,
            &self.canvas,
            (bounds.x(), bounds.y()),
        );
        // Single strong blur for the live preview.
        let blurred = sss_core::blur::gaussian_blur(cropped, 14.0);
        let (w, h) = (blurred.width() as usize, blurred.height() as usize);
        let pixels = egui::ColorImage::from_rgba_unmultiplied([w, h], blurred.as_raw());
        Some(ctx.load_texture(
            format!("sss_capture_ui::blur_src::{pos}"),
            pixels,
            egui::TextureOptions::LINEAR,
        ))
    }

    fn render_window(&mut self, id: WinitWindowId, _event_loop: &dyn ActiveEventLoop) {
        use crate::render::overlay::{draw_canvas, draw_confirm_hint};

        let gpu = match self.gpu.clone() {
            Some(g) => g,
            None => return,
        };
        let pos = match self.windows.iter().position(|w| w.window.id() == id) {
            Some(p) => p,
            None => return,
        };
        let (origin_x, origin_y, monitor_w, monitor_h) = {
            let m = &self.windows[pos].monitor;
            (
                m.bounds().x(),
                m.bounds().y(),
                m.bounds().width(),
                m.bounds().height(),
            )
        };

        // Take and re-insert window_gpu to split the borrow against `window`.
        let mut window_gpu = match self.windows[pos].gpu.take() {
            Some(g) => g,
            None => return,
        };
        let window_arc = self.windows[pos].window.clone();

        // Lazily upload this monitor's slice of the eager capture as an egui
        // texture so the overlay paints the desktop underneath the canvas.
        if self.windows[pos].background.is_none() {
            if let Some(tex) = self.build_monitor_background(pos, &window_gpu.egui_ctx) {
                self.windows[pos].background = Some(tex);
            }
        }
        // Bump canvas_version when the shape count changes. This is a
        // cheap proxy for "shapes added / removed / replaced" — gizmo
        // drags increment too via `replace_shape`, since the version is
        // checked against the cached one rather than directly observed.
        // (Shape moves without count change won't re-trigger, but those
        // happen during a drag where re-blurring every frame would be
        // too slow anyway; release triggers a final rebuild.)
        let cur_hash = self.canvas_state_hash();
        if cur_hash != self.canvas_version {
            self.canvas_version = cur_hash;
        }
        // Build / refresh the blur-source texture: this is bg + all
        // non-BlurRect shapes, gaussian-blurred. Only rebuilt when the
        // canvas mutates AND there's at least one BlurRect to consume it
        // (cheap path: skip the blur entirely when no blur is in use).
        let has_blur_rect = self.canvas.shapes().iter().any(|s| {
            matches!(s.kind, crate::shape::ShapeKind::BlurRect { .. })
        }) || self
            .canvas
            .preview_shape()
            .map(|s| matches!(s.kind, crate::shape::ShapeKind::BlurRect { .. }))
            .unwrap_or(false);
        if has_blur_rect {
            let needs_rebuild = match &self.windows[pos].blur_source {
                Some((v, _)) => *v != self.canvas_version,
                None => true,
            };
            if needs_rebuild {
                if let Some(tex) = self.build_blur_source(pos, &window_gpu.egui_ctx) {
                    self.windows[pos].blur_source = Some((self.canvas_version, tex));
                }
            }
        }
        let background = self.windows[pos].background.clone();
        let background_blurred = self
            .windows[pos]
            .blur_source
            .as_ref()
            .map(|(_, t)| t.clone());
        // Take per-window icon cache out so the ctx.run closure can
        // mutably borrow it without aliasing `self.windows`.
        let mut icons = std::mem::take(&mut self.windows[pos].icons);
        // Pre-decide if this overlay should host the toolbar — the overlay
        // with the largest overlap with the active region wins; fall back
        // to the focused overlay when there's no region.
        let host_toolbar = self.is_toolbar_host(pos);

        let raw_input = window_gpu.egui_winit.take_egui_input(&*window_arc);

        let mut confirm = false;
        let mut cancel = false;
        let mut local_chrome: Vec<egui::Rect> = Vec::new();
        let full_output = window_gpu.egui_ctx.clone().run(raw_input, |ctx| {
            // Background + canvas first so toolbar / radial sit on top.
            egui::CentralPanel::default()
                .frame(egui::Frame::new())
                .show(ctx, |ui| {
                    let screen_rect = ui.max_rect();
                    let painter = ui.painter();
                    let monitor_origin = egui::Pos2::new(origin_x as f32, origin_y as f32);
                    if let Some(tex) = background.as_ref() {
                        painter.image(
                            tex.id(),
                            screen_rect,
                            egui::Rect::from_min_max(
                                egui::Pos2::ZERO,
                                egui::Pos2::new(1.0, 1.0),
                            ),
                            egui::Color32::WHITE,
                        );
                    }
                    // Background dim: darken everything outside the active
                    // region (or whole monitor when no region) so the user
                    // visually focuses on the selection.
                    let dim_alpha = self.config.ui.background_dim;
                    if dim_alpha > 0 {
                        let dim_color =
                            egui::Color32::from_rgba_unmultiplied(0, 0, 0, dim_alpha);
                        let region = self.canvas.region();
                        if let Some(r) = region.filter(|r| r.width() >= 2 && r.height() >= 2)
                        {
                            let rx = r.x() as f32 - origin_x as f32;
                            let ry = r.y() as f32 - origin_y as f32;
                            let rw = r.width() as f32;
                            let rh = r.height() as f32;
                            let sr = screen_rect;
                            // Top.
                            painter.rect_filled(
                                egui::Rect::from_min_max(
                                    sr.min,
                                    egui::Pos2::new(sr.max.x, sr.min.y + ry.max(0.0)),
                                ),
                                0.0,
                                dim_color,
                            );
                            // Bottom.
                            painter.rect_filled(
                                egui::Rect::from_min_max(
                                    egui::Pos2::new(sr.min.x, sr.min.y + (ry + rh).max(0.0)),
                                    sr.max,
                                ),
                                0.0,
                                dim_color,
                            );
                            // Left strip.
                            painter.rect_filled(
                                egui::Rect::from_min_max(
                                    egui::Pos2::new(sr.min.x, sr.min.y + ry.max(0.0)),
                                    egui::Pos2::new(
                                        sr.min.x + rx.max(0.0),
                                        sr.min.y + (ry + rh).max(0.0),
                                    ),
                                ),
                                0.0,
                                dim_color,
                            );
                            // Right strip.
                            painter.rect_filled(
                                egui::Rect::from_min_max(
                                    egui::Pos2::new(
                                        sr.min.x + (rx + rw).max(0.0),
                                        sr.min.y + ry.max(0.0),
                                    ),
                                    egui::Pos2::new(sr.max.x, sr.min.y + (ry + rh).max(0.0)),
                                ),
                                0.0,
                                dim_color,
                            );
                        } else {
                            painter.rect_filled(screen_rect, 0.0, dim_color);
                        }
                    }
                    // Snap grid dots inside the active region so the user
                    // can see where points will snap to.
                    if self.snap_on {
                        if let Some(r) = self.canvas.region().filter(|r| {
                            r.width() >= 2 && r.height() >= 2
                        }) {
                            let step = self.snap_step.max(2.0);
                            let dot = egui::Color32::from_rgba_unmultiplied(
                                255, 255, 255, 80,
                            );
                            let mut gy = (r.y() as f32 / step).floor() * step;
                            let max_y = (r.y() + r.height() as i32) as f32;
                            let max_x = (r.x() + r.width() as i32) as f32;
                            while gy <= max_y {
                                let mut gx = (r.x() as f32 / step).floor() * step;
                                while gx <= max_x {
                                    let lx = gx - origin_x as f32;
                                    let ly = gy - origin_y as f32;
                                    painter.circle_filled(
                                        egui::Pos2::new(lx, ly),
                                        1.2,
                                        dot,
                                    );
                                    gx += step;
                                }
                                gy += step;
                            }
                        }
                    }
                    let region_col = {
                        let c = self.config.ui.region_outline_color;
                        egui::Color32::from_rgba_unmultiplied(
                            c.0[0], c.0[1], c.0[2], c.0[3],
                        )
                    };
                    draw_canvas(
                        painter,
                        &self.canvas,
                        monitor_origin,
                        Some(self.last_cursor),
                        background_blurred.as_ref(),
                        (monitor_w, monitor_h),
                        region_col,
                    );
                    // Transform gizmos: only paint on the overlay holding
                    // the largest slice of the selected shape bounds, so
                    // the handles don't repeat on every monitor.
                    if let Some(sel) = self.canvas.selected() {
                        if let Some(shape) =
                            self.canvas.shapes().iter().find(|s| s.id == sel)
                        {
                            let b = shape.bounds();
                            let is_owner = {
                                let mut best: (usize, u64) = (pos, 0);
                                for (i, w) in self.windows.iter().enumerate() {
                                    if let Some(inter) = w.monitor.bounds().intersection(&b) {
                                        let area =
                                            inter.width() as u64 * inter.height() as u64;
                                        if area > best.1 {
                                            best = (i, area);
                                        }
                                    }
                                }
                                best.0 == pos && best.1 > 0
                            };
                            if is_owner
                                && matches!(
                                    self.canvas.active_tool,
                                    crate::tool::Tool::Pointer
                                )
                            {
                                crate::render::ui::draw_gizmos(
                                    painter,
                                    b,
                                    monitor_origin,
                                    &self.config.ui.chrome,
                                );
                            }
                        }
                    }
                    if self.config.confirm_with_enter {
                        let drawing_poly = matches!(
                            self.canvas.active_tool,
                            crate::tool::Tool::Polygon(_)
                        );
                        let hint = if drawing_poly {
                            if self.canvas.is_drawing_polygon() {
                                "Right-click or Enter to close polygon"
                            } else {
                                "Click to add polygon vertices · right-click to close"
                            }
                        } else {
                            "Press Enter to accept"
                        };
                        draw_confirm_hint(
                            painter,
                            screen_rect,
                            self.canvas.region(),
                            monitor_origin,
                            monitor_w as f32,
                            hint,
                            &self.config.ui.chrome,
                        );
                    }
                });

            let mut main_tb_rect: Option<egui::Rect> = None;
            if self.config.toolbar && host_toolbar {
                let region = self.canvas.region();
                let (out, tb_rect) = crate::render::ui::draw_toolbar(
                    ctx,
                    &mut self.canvas,
                    &self.config.palette,
                    &mut self.runtime_mode,
                    self.current_color,
                    self.current_width,
                    egui::Pos2::new(origin_x as f32, origin_y as f32),
                    egui::Vec2::new(monitor_w as f32, monitor_h as f32),
                    region,
                    &self.config.ui.chrome,
                    crate::render::ui::ToolbarConfig {
                        pipette_active: self.pipette_pending,
                        snap_active: self.snap_on,
                        magnifier_active: self.magnifier_on,
                        snap_step: self.snap_step,
                    },
                    &mut icons,
                );
                main_tb_rect = Some(tb_rect);
                local_chrome.push(tb_rect);
                if let Some(i) = out.select_tool {
                    if let Some(tool) = self.config.palette.tools.get(i).cloned() {
                        self.canvas.set_tool(tool);
                        self.current_fill = None;
                        self.push_current_to_tool();
                    }
                }
                if let Some(i) = out.select_tool_filled {
                    if let Some(tool) = self.config.palette.tools.get(i).cloned() {
                        self.canvas.set_tool(tool);
                        self.current_fill = Some(self.current_color);
                        self.push_current_to_tool();
                    }
                }
                if out.undo {
                    self.canvas.handle(CanvasEvent::Undo);
                }
                if out.redo {
                    self.canvas.handle(CanvasEvent::Redo);
                }
                if out.clear_all {
                    self.canvas.clear_shapes();
                }
                if out.copy {
                    self.action.copy = true;
                }
                if out.save {
                    self.action.save = true;
                }
                if out.confirm {
                    confirm = true;
                }
                if out.cancel {
                    cancel = true;
                }
                if out.toggle_pipette {
                    self.pipette_pending = !self.pipette_pending;
                }
                if out.toggle_snap {
                    self.snap_on = !self.snap_on;
                }
                if out.toggle_magnifier {
                    self.magnifier_on = !self.magnifier_on;
                }
                if let Some(origin) = out.open_width_popup {
                    self.width_popup = if self.width_popup.is_some() {
                        None
                    } else {
                        Some((pos, origin, false))
                    };
                }
                if let Some(origin) = out.open_snap_popup {
                    self.snap_popup = if self.snap_popup.is_some() {
                        None
                    } else {
                        Some((pos, origin, false))
                    };
                }
                if let Some(origin) = out.open_color_popup {
                    self.color_popup = if self.color_popup.is_some() {
                        None
                    } else {
                        Some((
                            pos,
                            origin,
                            false,
                            crate::render::ui::HsvState::from_rgb(self.current_color),
                        ))
                    };
                }
                // raise/lower/delete now live on the selection toolbar, not
                // the main one. Outcome fields kept for API stability.
                let _ = (out.raise_selected, out.lower_selected, out.delete_selected);
            }

            // Side action toolbar (undo / redo / clear / confirm / copy /
            // save / cancel) — vertical, anchored to the region edge. Only
            // shown on the same overlay the main toolbar lives on so the
            // chips don't repeat per monitor.
            if self.config.toolbar && host_toolbar {
                if let Some(region) = self
                    .canvas
                    .region()
                    .filter(|r| r.width() >= 2 && r.height() >= 2)
                {
                    let (out, act_rect) = crate::render::ui::draw_action_toolbar(
                        ctx,
                        region,
                        egui::Pos2::new(origin_x as f32, origin_y as f32),
                        egui::Vec2::new(monitor_w as f32, monitor_h as f32),
                        main_tb_rect,
                        &self.config.ui.chrome,
                        crate::render::ui::ActionToolbarConfig {
                            show_copy: self.config.show_copy,
                            show_save: self.config.show_save,
                        },
                        &mut icons,
                    );
                    local_chrome.push(act_rect);
                    if out.undo {
                        self.canvas.handle(CanvasEvent::Undo);
                    }
                    if out.redo {
                        self.canvas.handle(CanvasEvent::Redo);
                    }
                    if out.clear_all {
                        self.canvas.clear_shapes();
                    }
                    if out.copy {
                        self.action.copy = true;
                        confirm = true;
                    }
                    if out.save {
                        self.action.save = true;
                        confirm = true;
                    }
                    if out.confirm {
                        confirm = true;
                    }
                    if out.cancel {
                        cancel = true;
                    }
                }
            }

            // Selection toolbar (raise / lower / trash) — anchored to the
            // selected shape's bounds on the overlay with the largest
            // intersection.
            if matches!(self.canvas.active_tool, crate::tool::Tool::Pointer) {
                if let Some(sel) = self.canvas.selected() {
                    if let Some(shape) =
                        self.canvas.shapes().iter().find(|s| s.id == sel)
                    {
                        let b = shape.bounds();
                        let is_owner = {
                            let mut best: (usize, u64) = (pos, 0);
                            for (i, w) in self.windows.iter().enumerate() {
                                if let Some(inter) = w.monitor.bounds().intersection(&b) {
                                    let area =
                                        inter.width() as u64 * inter.height() as u64;
                                    if area > best.1 {
                                        best = (i, area);
                                    }
                                }
                            }
                            best.0 == pos && best.1 > 0
                        };
                        if is_owner {
                            let (sel_out, sel_rect) =
                                crate::render::ui::draw_selection_toolbar(
                                    ctx,
                                    b,
                                    egui::Pos2::new(origin_x as f32, origin_y as f32),
                                    egui::Vec2::new(
                                        monitor_w as f32,
                                        monitor_h as f32,
                                    ),
                                    &self.config.ui.chrome,
                                    &mut icons,
                                );
                            local_chrome.push(sel_rect);
                            if sel_out.raise {
                                self.canvas.raise_selected();
                            }
                            if sel_out.lower {
                                self.canvas.lower_selected();
                            }
                            if sel_out.delete {
                                self.canvas.handle(CanvasEvent::Delete);
                            }
                        }
                    }
                }
            }

            // Width popup.
            if let Some((p_pos, origin, mut armed)) = self.width_popup {
                if p_pos == pos {
                    let (out, w_rect) = crate::render::ui::draw_slider_popup(
                        ctx,
                        "width",
                        origin,
                        &mut armed,
                        "Width",
                        1.0,
                        40.0,
                        self.current_width,
                        &self.config.ui.chrome,
                    );
                    if let Some(slot) = self.width_popup.as_mut() {
                        slot.2 = armed;
                    }
                    local_chrome.push(w_rect);
                    if let Some(v) = out.value {
                        self.apply_width_pick(v);
                    }
                    if out.close {
                        self.width_popup = None;
                    }
                }
            }

            // Color picker popup.
            if let Some((p_pos, origin, armed_in, state_in)) =
                self.color_popup.clone()
            {
                if p_pos == pos {
                    let mut armed = armed_in;
                    let mut state = state_in;
                    let (out, c_rect) = crate::render::ui::draw_color_popup(
                        ctx,
                        origin,
                        &mut armed,
                        &mut state,
                        &self.config.ui.chrome,
                    );
                    local_chrome.push(c_rect);
                    // Persist state + armed between frames.
                    if let Some(slot) = self.color_popup.as_mut() {
                        slot.2 = armed;
                        slot.3 = state;
                    }
                    if let Some(c) = out.color {
                        self.apply_color_pick(c, false);
                    }
                    if out.close {
                        self.color_popup = None;
                    }
                }
            }

            // Snap step popup.
            if let Some((p_pos, origin, mut armed)) = self.snap_popup {
                if p_pos == pos {
                    let (out, s_rect) = crate::render::ui::draw_slider_popup(
                        ctx,
                        "snap",
                        origin,
                        &mut armed,
                        "Snap step",
                        2.0,
                        100.0,
                        self.snap_step,
                        &self.config.ui.chrome,
                    );
                    if let Some(slot) = self.snap_popup.as_mut() {
                        slot.2 = armed;
                    }
                    local_chrome.push(s_rect);
                    if let Some(v) = out.value {
                        self.snap_step = v.max(2.0);
                    }
                    if out.close {
                        self.snap_popup = None;
                    }
                }
            }

            // Magnifier overlay (drawn only on the overlay holding the
            // pointer). Uses this monitor's background texture as the source
            // surface, sampling around the pointer in window-local pixels.
            if self.magnifier_on || self.pipette_pending {
                if let (Some(tex), Some(focused_id)) =
                    (background.as_ref(), self.active_window)
                {
                    if focused_id == id {
                        let ptr_local = egui::Pos2::new(
                            self.last_cursor.x - origin_x as f32,
                            self.last_cursor.y - origin_y as f32,
                        );
                        let hex = self.initial.as_ref().and_then(|img| {
                            let bb = sss_capture::Rect::bounding(
                                &self
                                    .monitors
                                    .iter()
                                    .map(|m| m.bounds())
                                    .collect::<Vec<_>>(),
                            )
                            .unwrap_or_default();
                            crate::render::ui::sample_capture(
                                img,
                                (bb.x(), bb.y()),
                                (
                                    self.last_cursor.x as i32,
                                    self.last_cursor.y as i32,
                                ),
                            )
                            .map(|c| {
                                format!(
                                    "#{:02X}{:02X}{:02X}",
                                    c.0[0], c.0[1], c.0[2]
                                )
                            })
                        });
                        crate::render::ui::draw_magnifier(
                            ctx,
                            tex,
                            ptr_local,
                            egui::Vec2::new(monitor_w as f32, monitor_h as f32),
                            4.0,
                            &self.config.ui.chrome,
                            hex,
                        );
                    }
                }
            }

            // Radial menu — only drawn on the overlay it was opened on.
            if let Some((radial_pos, state, mut armed)) = self.radial {
                if radial_pos == pos {
                    let (outcome, r_rect) = crate::render::ui::draw_radial(
                        ctx,
                        &state,
                        &mut armed,
                        &self.config.palette.color_palette,
                        &self.config.ui.radial_widths,
                        self.current_color,
                        self.current_width,
                        &self.config.ui.chrome,
                    );
                    if let Some(slot) = self.radial.as_mut() {
                        slot.2 = armed;
                    }
                    local_chrome.push(r_rect);
                    if let Some(pick) = outcome.pick {
                        match pick {
                            crate::render::ui::RadialPick::Color(c) => {
                                self.apply_color_pick(c, false);
                            }
                            crate::render::ui::RadialPick::Width(w) => {
                                self.apply_width_pick(w);
                            }
                        }
                        self.radial = None;
                    } else if outcome.close {
                        self.radial = None;
                    }
                }
            }
        });

        // Stash chrome rects so the event router can hit-test pointer
        // clicks against them on the next event.
        if local_chrome.is_empty() {
            self.chrome_rects.remove(&pos);
        } else {
            self.chrome_rects.insert(pos, local_chrome);
        }
        self.windows[pos].icons = icons;

        window_gpu
            .egui_winit
            .handle_platform_output(&*window_arc, full_output.platform_output.clone());

        let primitives = window_gpu
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        let screen_desc = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [window_gpu.size.0, window_gpu.size.1],
            pixels_per_point: full_output.pixels_per_point,
        };

        for (id, image_delta) in &full_output.textures_delta.set {
            window_gpu
                .renderer
                .update_texture(&gpu.device, &gpu.queue, *id, image_delta);
        }

        let output = match window_gpu.surface.get_current_texture() {
            Ok(surface_texture) => surface_texture,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                window_gpu
                    .surface
                    .configure(&gpu.device, &window_gpu.config);
                self.windows[pos].gpu = Some(window_gpu);
                return;
            }
            Err(e) => {
                tracing::warn!(error = %e, "wgpu: get_current_texture failed");
                self.windows[pos].gpu = Some(window_gpu);
                return;
            }
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("sss_capture_ui encoder"),
            });
        window_gpu.renderer.update_buffers(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &primitives,
            &screen_desc,
        );
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sss_capture_ui pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Opaque (alpha=1.0): per-pixel compositor blending
                        // against the desktop is expensive on multi-monitor
                        // setups and has triggered GPU-driver kernel crashes
                        // on niri with 4 outputs.
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let pass = &mut pass.forget_lifetime();
            window_gpu.renderer.render(pass, &primitives, &screen_desc);
        }
        gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        for id in &full_output.textures_delta.free {
            window_gpu.renderer.free_texture(id);
        }

        self.windows[pos].gpu = Some(window_gpu);

        if confirm {
            self.confirm(_event_loop);
        } else if cancel {
            self.outcome = Some(Outcome::Cancelled);
            self.flush_and_exit(_event_loop);
        }
    }
}
