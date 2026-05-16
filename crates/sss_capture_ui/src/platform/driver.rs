//! Top-level driver: connects [`crate::Selector`] to `winit`.
//!
//! Two execution paths:
//!
//! * **`editor` feature off** — minimal selection-only path. Boots a fullscreen
//!   `winit` window per monitor, paints the captured screenshot as background,
//!   tracks the rubber-band rectangle and returns the result. Equivalent to
//!   `slurp`.
//! * **`editor` feature on** — same windowing layer, plus an egui toolbar
//!   stacked on top with every annotation tool wired into [`crate::Canvas`].
//!
//! The driver always runs the canvas state machine and always renders shapes
//! over the captured frame at confirm time through [`crate::render::composite`].

use std::sync::Arc;

use sss_capture::Image as CapImage;
use sss_capture::Monitor;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
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
    // 1) Snapshot the desktop now (Eager) so we have something to paint
    //    behind the toolbar. Lazy mode skips this and grabs at confirm time.
    //
    // Capture failure is non-fatal here: when the OS / compositor refuses
    // (Wayland portal timeout, GNOME consent dialog dismissed, …) we still
    // want to show the GUI so the user can pick a region and we retry the
    // capture on confirm. The background just stays empty until then.
    let initial = if matches!(config.trigger, CaptureTrigger::Eager) {
        match capturer.capture_all_with(config.capture_opts) {
            Ok(img) => Some(img),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "initial eager capture failed; opening the selector \
                     with no background (capture will retry on confirm)",
                );
                eprintln!(
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

    let monitors = capturer.monitors().map_err(SelectorError::Capture)?;

    let event_loop =
        EventLoop::new().map_err(|e| SelectorError::Backend(format!("winit event loop: {e}")))?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);

    let save_path_hint = config.save_path_hint.clone();
    let initial_mode = match config.mode {
        SelectorMode::AnyOf => SelectorMode::Area,
        m => m,
    };
    let mut app = App {
        config,
        capturer,
        monitors,
        initial,
        windows: Vec::new(),
        canvas: Canvas::new(),
        active_window: None,
        last_cursor: FPoint::default(),
        outcome: None,
        action: PostAction {
            copy: false,
            save: false,
            save_path_hint,
        },
        mods: ModState::default(),
        #[cfg(feature = "editor")]
        gpu: None,
        runtime_mode: initial_mode,
    };

    event_loop
        .run_app(&mut app)
        .map_err(|e| SelectorError::Backend(format!("event loop: {e}")))?;

    let outcome = app.outcome.unwrap_or(Outcome::Cancelled);
    Ok(Selection {
        outcome,
        canvas: app.canvas,
        action: app.action,
    })
}

#[derive(Default, Clone, Copy, Debug)]
struct ModState {
    ctrl: bool,
    shift: bool,
    alt: bool,
    meta: bool,
}

// ---------------------------------------------------------------------------
// ApplicationHandler
// ---------------------------------------------------------------------------

struct App {
    config: crate::selector::Config,
    capturer: Arc<sss_capture::Capturer>,
    monitors: Vec<Monitor>,
    initial: Option<CapImage>,
    windows: Vec<OverlayWindow>,
    canvas: Canvas,
    active_window: Option<WinitWindowId>,
    last_cursor: FPoint,
    outcome: Option<Outcome>,
    action: PostAction,
    mods: ModState,
    #[cfg(feature = "editor")]
    gpu: Option<Arc<crate::render::gpu::Gpu>>,
    /// Tracks SelectorMode at runtime — `AnyOf` lets the user switch tabs.
    runtime_mode: SelectorMode,
}

struct OverlayWindow {
    window: Arc<Window>,
    monitor: Monitor,
    #[cfg(feature = "editor")]
    gpu: Option<crate::render::gpu::WindowGpu>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        tracing::info!("App::resumed — creating overlay windows");
        if !self.windows.is_empty() {
            tracing::debug!("resume after suspend; reusing existing windows");
            return;
        }
        let winit_monitors: Vec<_> = event_loop.available_monitors().collect();
        tracing::info!(
            "winit reports {} available monitor(s); sss_capture reports {}",
            winit_monitors.len(),
            self.monitors.len()
        );
        for (i, monitor) in self.monitors.iter().enumerate() {
            // Build a fullscreen attribute set targeting the winit MonitorHandle
            // whose position matches the sss_capture monitor.
            let target = winit_monitors.iter().find(|m| {
                let pos = m.position();
                pos.x == monitor.bounds().x() && pos.y == monitor.bounds().y()
            });
            // Fall back to `Borderless(None)` so the compositor picks the
            // current output even when winit cannot enumerate them (a known
            // wart on some Wayland setups).
            let fullscreen = Some(winit::window::Fullscreen::Borderless(target.cloned()));
            tracing::info!(
                monitor = %monitor.name(),
                index = i,
                handle_matched = target.is_some(),
                bounds = %monitor.bounds(),
                "creating overlay window",
            );
            let attrs = Window::default_attributes()
                .with_title("sss_capture_ui overlay")
                .with_decorations(false)
                .with_resizable(false)
                .with_visible(true)
                .with_active(true)
                // Explicit inner_size — winit Wayland sometimes opens a 0×0
                // window when neither inner_size nor a configured fullscreen
                // is set, which the compositor then hides.
                .with_inner_size(winit::dpi::PhysicalSize::new(
                    monitor.bounds().width().max(640),
                    monitor.bounds().height().max(480),
                ))
                .with_transparent(matches!(self.config.trigger, CaptureTrigger::Lazy { .. }))
                .with_fullscreen(fullscreen);
            match event_loop.create_window(attrs) {
                Ok(window) => {
                    let window = Arc::new(window);
                    let id = window.id();
                    // Kick off the first frame — `ControlFlow::Wait` would
                    // otherwise sit forever until the user moved the cursor.
                    window.request_redraw();
                    tracing::info!(?id, "overlay window created and redraw requested");
                    let overlay = OverlayWindow {
                        window,
                        monitor: monitor.clone(),
                        #[cfg(feature = "editor")]
                        gpu: None,
                    };
                    self.windows.push(overlay);
                }
                Err(e) => {
                    eprintln!(
                        "sss_capture_ui: failed to create overlay window for {}: {e}",
                        monitor
                    );
                    tracing::error!(error = %e, "failed to create overlay window for {monitor}");
                }
            }
        }
        tracing::info!("opened {} overlay window(s)", self.windows.len());

        // Initialise GPU state once we have a window with a valid handle.
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

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
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
        self.active_window = Some(id);

        match event {
            WindowEvent::CloseRequested => {
                self.outcome = Some(Outcome::Cancelled);
                event_loop.exit();
            }
            WindowEvent::ModifiersChanged(state) => {
                let m = state.state();
                self.mods.ctrl = m.control_key();
                self.mods.shift = m.shift_key();
                self.mods.alt = m.alt_key();
                self.mods.meta = m.super_key();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                match event.logical_key.as_ref() {
                    Key::Named(NamedKey::Escape) => {
                        self.outcome = Some(Outcome::Cancelled);
                        event_loop.exit();
                    }
                    Key::Named(NamedKey::Enter) if self.config.confirm_with_enter => {
                        self.confirm(event_loop);
                    }
                    Key::Named(NamedKey::Backspace) => {
                        self.canvas.handle(CanvasEvent::TextBackspace);
                    }
                    // Ctrl+Z / Ctrl+Shift+Z = undo / redo
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
                    // Ctrl+C — copy intent, then confirm.
                    Key::Character("c") | Key::Character("C") if self.mods.ctrl => {
                        self.action.copy = true;
                        self.confirm(event_loop);
                    }
                    // Ctrl+S — save intent, then confirm.
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
            WindowEvent::CursorMoved { position, .. } => {
                let p = FPoint::new(
                    position.x as f32 + origin.0 as f32,
                    position.y as f32 + origin.1 as f32,
                );
                self.last_cursor = p;
                self.canvas.handle(CanvasEvent::PointerMove(p));
                if let Some(win) = self.windows.iter().find(|w| w.window.id() == id) {
                    win.window.request_redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button != MouseButton::Left {
                    return;
                }
                match state {
                    ElementState::Pressed => {
                        self.canvas
                            .handle(CanvasEvent::PointerDown(self.last_cursor));
                    }
                    ElementState::Released => {
                        self.canvas.handle(CanvasEvent::PointerUp(self.last_cursor));
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                tracing::trace!(?id, "redraw requested");
                #[cfg(feature = "editor")]
                self.render_window(id, event_loop);
                #[cfg(not(feature = "editor"))]
                {
                    // No GPU backend compiled — the user sees their normal
                    // desktop with the system cursor and the rubber-band is
                    // computed in software but invisible. Hosts that need a
                    // visual rectangle should build with `--features editor`.
                }
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
            WindowEvent::Resized(new_size) => {
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
    fn confirm(&mut self, event_loop: &ActiveEventLoop) {
        // Build the outcome based on the (runtime) mode + canvas state.
        let region = self.canvas.region();
        let outcome = match self.runtime_mode {
            SelectorMode::Monitor => {
                // Pick the monitor under the cursor at confirm time.
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
                // The minimal driver doesn't draw window thumbnails; fall back
                // to the foreground window under the cursor.
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
        event_loop.exit();
    }

    /// Materialise the captured image for `rect`. In Eager mode we crop the
    /// pre-grabbed full-desktop screenshot and overlay shapes. In Lazy mode
    /// we ask the capturer right now.
    fn capture_region(&self, rect: sss_capture::Rect) -> Option<CapImage> {
        let raw = match self.initial.clone() {
            Some(img) => {
                // Crop from the pre-captured full desktop.
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
        // Bake shapes onto the cropped image.
        crate::render::composite::flatten(&mut buf, &self.canvas, (rect.x(), rect.y()));
        Some(CapImage::new(buf))
    }
}

// ---------------------------------------------------------------------------
// Editor-feature: egui + wgpu rendering
// ---------------------------------------------------------------------------

#[cfg(feature = "editor")]
impl App {
    fn init_gpu(&mut self) {
        // Build wgpu's instance + adapter + device + per-window state.
        //
        // wgpu 22 keeps an internal reference to the `compatible_surface`
        // passed to `request_adapter` for as long as the adapter lives, so
        // we MUST NOT drop that surface ahead of time. The flow below
        // creates the first window's surface, hands it to the adapter, and
        // then *moves it* into the first `WindowGpu` (via
        // `WindowGpu::from_surface`) so it stays alive for the full
        // session.
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
                eprintln!("sss_capture_ui: wgpu surface creation failed: {e}");
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
                eprintln!("sss_capture_ui: wgpu init failed: {e}");
                tracing::error!(error = %e, "wgpu init failed; editor disabled");
                return;
            }
        };

        // Attach the surface we already have to the first window.
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
                eprintln!("sss_capture_ui: window-0 wgpu init failed: {e}");
                tracing::error!(error = %e, "wgpu: window-0 init failed");
                // Without window 0 the GPU is unusable; bail out cleanly.
                return;
            }
        }

        // Create surfaces + per-window state for the rest.
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
                    eprintln!("sss_capture_ui: per-window wgpu init failed (window {idx}): {e}");
                    tracing::warn!(error = %e, window = idx, "wgpu: per-window init failed");
                }
            }
        }
        self.gpu = Some(gpu);
    }

    fn render_window(&mut self, id: WinitWindowId, _event_loop: &ActiveEventLoop) {
        use crate::render::overlay::{draw_canvas, draw_toolbar, ToolbarConfig};

        let gpu = match self.gpu.clone() {
            Some(g) => g,
            None => return,
        };
        // Borrow split: pull out the OverlayWindow we render, leave others.
        let pos = match self.windows.iter().position(|w| w.window.id() == id) {
            Some(p) => p,
            None => return,
        };
        let (origin_x, origin_y) = {
            let m = &self.windows[pos].monitor;
            (m.bounds().x(), m.bounds().y())
        };

        // We need disjoint borrows of `windows[pos].gpu` (mut) and the egui
        // input from the `window` (ref). Take ownership of `gpu` slot to
        // sidestep the borrow checker, then put it back.
        let mut window_gpu = match self.windows[pos].gpu.take() {
            Some(g) => g,
            None => return,
        };
        let window_arc = self.windows[pos].window.clone();

        // 1) Build raw input from winit state.
        let raw_input = window_gpu.egui_winit.take_egui_input(&*window_arc);

        // 2) egui frame -------------------------------------------------------
        let mut confirm = false;
        let mut cancel = false;
        let full_output = window_gpu.egui_ctx.clone().run(raw_input, |ctx| {
            // Toolbar
            if self.config.toolbar {
                let out = draw_toolbar(
                    ctx,
                    &mut self.canvas,
                    &self.config.palette,
                    &mut self.runtime_mode,
                    ToolbarConfig {
                        show_copy: self.config.show_copy,
                        show_save: self.config.show_save,
                    },
                );
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
            }
            // Shape canvas — drawn into the egui central panel so we cover
            // the whole client area of the window.
            egui::CentralPanel::default()
                .frame(egui::Frame::none())
                .show(ctx, |ui| {
                    let painter = ui.painter();
                    draw_canvas(
                        painter,
                        &self.canvas,
                        egui::Pos2::new(origin_x as f32, origin_y as f32),
                    );
                });
        });

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

        // 3) Upload textures and render through wgpu.
        for (id, image_delta) in &full_output.textures_delta.set {
            window_gpu
                .renderer
                .update_texture(&gpu.device, &gpu.queue, *id, image_delta);
        }

        let output = match window_gpu.surface.get_current_texture() {
            Ok(t) => t,
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
                        // Opaque clear color: `background_dim` controls how
                        // dark the dim is, but `a` is always 1.0. Letting
                        // the surface alpha float means the compositor has
                        // to per-pixel-blend the overlay against the
                        // desktop on every frame, which is expensive on
                        // multi-monitor setups and (in practice) GPU-driver
                        // unsafe — we saw a kernel-side null deref the
                        // first time this overlay went live on a 4-monitor
                        // niri session.
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

        // Put the gpu state back.
        self.windows[pos].gpu = Some(window_gpu);

        // Apply the side effects from the toolbar.
        if confirm {
            self.confirm(_event_loop);
        } else if cancel {
            self.outcome = Some(Outcome::Cancelled);
            _event_loop.exit();
        }
    }
}
