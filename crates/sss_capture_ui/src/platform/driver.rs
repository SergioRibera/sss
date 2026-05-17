//! Top-level winit-based driver, with optional egui editor toolbar.

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
        canvas: Canvas::default(),
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
            let target = winit_monitors.iter().find(|m| {
                let pos = m.position();
                pos.x == monitor.bounds().x() && pos.y == monitor.bounds().y()
            });
            // `Borderless(None)` lets the compositor pick the current output
            // when winit can't enumerate (some Wayland setups).
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
                // Without an explicit inner_size winit-Wayland can open a 0x0
                // window that the compositor then hides.
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
                        self.action.copy = true;
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
        event_loop.exit();
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

    fn render_window(&mut self, id: WinitWindowId, _event_loop: &ActiveEventLoop) {
        use crate::render::overlay::{draw_canvas, draw_confirm_hint, draw_toolbar, ToolbarConfig};

        let gpu = match self.gpu.clone() {
            Some(g) => g,
            None => return,
        };
        let pos = match self.windows.iter().position(|w| w.window.id() == id) {
            Some(p) => p,
            None => return,
        };
        let (origin_x, origin_y, monitor_w) = {
            let m = &self.windows[pos].monitor;
            (m.bounds().x(), m.bounds().y(), m.bounds().width())
        };

        // Take and re-insert window_gpu to split the borrow against `window`.
        let mut window_gpu = match self.windows[pos].gpu.take() {
            Some(g) => g,
            None => return,
        };
        let window_arc = self.windows[pos].window.clone();

        let raw_input = window_gpu.egui_winit.take_egui_input(&window_arc);

        let mut confirm = false;
        let mut cancel = false;
        let full_output = window_gpu.egui_ctx.clone().run_ui(raw_input, |ctx| {
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
            egui::CentralPanel::default()
                .frame(egui::Frame::new())
                .show_inside(ctx, |ui| {
                    let screen_rect = ui.max_rect();
                    let painter = ui.painter();
                    let monitor_origin = egui::Pos2::new(origin_x as f32, origin_y as f32);
                    draw_canvas(painter, &self.canvas, monitor_origin);
                    if self.config.confirm_with_enter {
                        draw_confirm_hint(
                            painter,
                            screen_rect,
                            self.canvas.region(),
                            monitor_origin,
                            monitor_w as f32,
                        );
                    }
                });
        });

        window_gpu
            .egui_winit
            .handle_platform_output(&window_arc, full_output.platform_output.clone());

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
            wgpu::CurrentSurfaceTexture::Success(surface_texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(surface_texture) => surface_texture,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                window_gpu
                    .surface
                    .configure(&gpu.device, &window_gpu.config);
                self.windows[pos].gpu = Some(window_gpu);
                return;
            }
            e => {
                let e = format!("{e:?}");
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
                    depth_slice: None,
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
                multiview_mask: None,
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
            _event_loop.exit();
        }
    }
}
