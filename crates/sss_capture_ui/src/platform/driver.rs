//! GPUI-backed driver: one root view per output, with a shared canvas
//! state observed by every window. On Linux Wayland the windows are
//! `WindowKind::LayerShell` surfaces; everywhere else they fall back to a
//! borderless fullscreen window.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{
    App, AppContext, Application, Bounds, Context, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Point, Render, StatefulInteractiveElement, Styled,
    Window, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, div, hsla, point,
    prelude::FluentBuilder, px, size, white,
};
use sss_capture::{Image as CapImage, Monitor, Rect as CapRect};

use crate::canvas::{Canvas, CanvasEvent};
use crate::color::Color;
use crate::geometry::FPoint;
use crate::mode::SelectorMode;
use crate::render::overlay::{
    BlurCache, Xform, paint_blurs, paint_canvas, paint_confirm_hint, paint_hover_target,
    paint_outside_mask,
};
use crate::selector::{Config, Outcome, PostAction, Selection, Selector, SelectorError};

// ─── Public entry point ─────────────────────────────────────────────────

pub fn run(sel: Selector) -> Result<Selection, SelectorError> {
    let Selector { config, capturer } = sel;

    let initial = match capturer.capture_all_with(config.capture_opts) {
        Ok(img) => Some(Arc::new(img)),
        Err(e) => {
            tracing::warn!(error = %e, "eager capture failed; overlay opens blank");
            None
        }
    };
    let monitors = capturer.monitors().map_err(SelectorError::Capture)?;
    let monitors_bb = CapRect::bounding(&monitors.iter().map(|m| m.bounds()).collect::<Vec<_>>())
        .unwrap_or_default();

    let save_path_hint = config.save_path_hint.clone();
    let initial_mode = match config.mode {
        SelectorMode::AnyOf => SelectorMode::Area,
        m => m,
    };

    let result_slot: Arc<Mutex<Option<Selection>>> = Arc::new(Mutex::new(None));

    let app: Application = gpui_platform::application().with_assets(crate::assets::UiAssets);
    {
        let monitors = monitors.clone();
        let result_slot = result_slot.clone();
        app.run(move |cx| {
            let windows = capturer.windows().unwrap_or_default();
            let shared = cx.new(|_| SharedState {
                canvas: Canvas::default(),
                runtime_mode: initial_mode,
                action: PostAction {
                    copy: false,
                    save: false,
                    save_path_hint,
                },
                last_cursor: FPoint::default(),
                outcome: None,
                config: Arc::new(config),
                capturer,
                initial,
                monitors: monitors.clone(),
                monitors_bb,
                windows,
                tool_before_pipette: None,
                result_slot,
            });

            // Match each sss_capture monitor to a GPUI display by origin.
            // On Wayland the display_id pins the layer-shell surface to the
            // right output; on macOS/X11 it picks the fullscreen target.
            //
            // `cx.displays()` is in logical pixels (DIPs); sss_capture is in
            // physical pixels. With scale = 1.0 they match; if not, the
            // origin tolerance below should still catch them. As a last
            // resort we fall back to matching by index (compositors
            // typically enumerate outputs in the same DRM order as
            // sss_capture's backend).
            let displays = cx.displays();
            tracing::info!(
                "gpui sees {} display(s); sss_capture sees {} monitor(s)",
                displays.len(),
                monitors.len()
            );
            for d in &displays {
                tracing::debug!("  gpui display id={:?} bounds={:?}", d.id(), d.bounds());
            }
            for m in monitors.iter() {
                tracing::debug!("  capture monitor id={} bounds={}", m.id(), m.bounds());
            }
            let mut opened = 0usize;
            for (idx, monitor) in monitors.iter().enumerate() {
                let m_bounds = monitor.bounds();
                let display_id = displays
                    .iter()
                    .find(|d| {
                        let b = d.bounds();
                        (b.origin.x.as_f32() as i32 - m_bounds.x()).abs() < 4
                            && (b.origin.y.as_f32() as i32 - m_bounds.y()).abs() < 4
                    })
                    .or_else(|| displays.get(idx))
                    .map(|d| d.id());
                tracing::debug!(
                    "  monitor {} ({}) -> display_id {:?}",
                    monitor.id(),
                    m_bounds,
                    display_id
                );

                let shared_for_window = shared.clone();
                let monitor_clone = monitor.clone();
                let opts = window_options_for(monitor, display_id, true);

                let attempt = cx.open_window(opts, {
                    let shared_for_window = shared_for_window.clone();
                    let monitor_clone = monitor_clone.clone();
                    move |window, cx| {
                        cx.new(|cx| {
                            OverlayView::new(window, cx, shared_for_window, monitor_clone)
                        })
                    }
                });
                match attempt {
                    Ok(_) => {
                        opened += 1;
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            monitor = %m_bounds,
                            "layer-shell open failed; retrying with a fullscreen xdg window"
                        );
                    }
                }

                // Compositor likely doesn't expose wlr-layer-shell (e.g.
                // GNOME Mutter, KDE without the protocol). Retry with a
                // regular fullscreen surface so the editor still works.
                let opts = window_options_for(monitor, display_id, false);
                match cx.open_window(opts, move |window, cx| {
                    cx.new(|cx| OverlayView::new(window, cx, shared_for_window, monitor_clone))
                }) {
                    Ok(_) => opened += 1,
                    Err(e) => tracing::error!(
                        error = %e,
                        monitor = %m_bounds,
                        "fullscreen overlay open failed"
                    ),
                }
            }

            if opened == 0 {
                tracing::error!(
                    "no overlay window could be opened on any monitor; closing"
                );
                shared.update(cx, |s, _| s.cancel());
                cx.quit();
                return;
            }
            tracing::info!(opened, "overlay ready");

            cx.on_window_closed(|cx, _id| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        });
    }

    let mut slot = result_slot.lock().unwrap();
    Ok(slot.take().unwrap_or_else(|| Selection {
        outcome: Outcome::Cancelled,
        canvas: Canvas::default(),
        action: PostAction::default(),
    }))
}

fn window_options_for(
    monitor: &Monitor,
    display_id: Option<gpui::DisplayId>,
    use_layer_shell: bool,
) -> WindowOptions {
    let m = monitor.bounds();
    let bounds = Bounds {
        origin: point(px(m.x() as f32), px(m.y() as f32)),
        size: size(px(m.width() as f32), px(m.height() as f32)),
    };

    let kind = if use_layer_shell {
        layer_shell_kind()
    } else {
        WindowKind::Normal
    };

    WindowOptions {
        window_bounds: Some(WindowBounds::Fullscreen(bounds)),
        titlebar: None,
        focus: true,
        show: true,
        kind,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("sss_capture_ui".into()),
        window_min_size: None,
        window_decorations: Some(gpui::WindowDecorations::Client),
        icon: None,
        tabbing_identifier: None,
    }
}

// `gpui::layer_shell` is gated to `cfg(all(target_os = "linux", feature =
// "wayland"))` inside gpui itself; we always enable the wayland feature on
// our Linux/macOS-only build, so on Linux we can address it unconditionally.
#[cfg(target_os = "linux")]
fn layer_shell_kind() -> WindowKind {
    use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
    WindowKind::LayerShell(LayerShellOptions {
        namespace: "sss_capture_ui".into(),
        layer: Layer::Overlay,
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        keyboard_interactivity: KeyboardInteractivity::Exclusive,
        ..Default::default()
    })
}

#[cfg(not(target_os = "linux"))]
fn layer_shell_kind() -> WindowKind {
    WindowKind::Normal
}

// ─── Shared canvas state ────────────────────────────────────────────────

struct SharedState {
    canvas: Canvas,
    runtime_mode: SelectorMode,
    action: PostAction,
    last_cursor: FPoint,
    outcome: Option<Outcome>,
    config: Arc<Config>,
    capturer: Arc<sss_capture::Capturer>,
    initial: Option<Arc<CapImage>>,
    monitors: Vec<Monitor>,
    monitors_bb: CapRect,
    windows: Vec<sss_capture::Window>,
    /// Tool the user was on before entering Pipette mode, so we can
    /// restore it once a colour is sampled (or pipette is cancelled).
    tool_before_pipette: Option<crate::tool::Tool>,
    result_slot: Arc<Mutex<Option<Selection>>>,
}

impl SharedState {
    fn hovered_window(&self) -> Option<&sss_capture::Window> {
        let p = sss_capture::Point::new(
            self.last_cursor.x as i32,
            self.last_cursor.y as i32,
        );
        self.windows.iter().find(|w| w.bounds().contains(p))
    }

    fn hovered_monitor(&self) -> Option<&Monitor> {
        let p = sss_capture::Point::new(
            self.last_cursor.x as i32,
            self.last_cursor.y as i32,
        );
        self.monitors.iter().find(|m| m.bounds().contains(p))
    }

    /// Sample the colour at the global pixel `p` from the eager capture,
    /// or `None` if there is no capture or the point is outside it.
    fn sample_color_at(&self, p: FPoint) -> Option<Color> {
        let img = self.initial.as_deref()?;
        let buf = img.as_rgba();
        let (w, h) = buf.dimensions();
        let x = (p.x as i32 - self.monitors_bb.x()).max(0) as u32;
        let y = (p.y as i32 - self.monitors_bb.y()).max(0) as u32;
        if x >= w || y >= h {
            return None;
        }
        let px = buf.get_pixel(x, y).0;
        Some(Color(px))
    }
}

impl SharedState {
    fn handle_canvas(&mut self, ev: CanvasEvent) {
        self.canvas.handle(ev);
    }

    fn confirm(&mut self) {
        let outcome = match self.runtime_mode {
            SelectorMode::Monitor => {
                if let Some(m) = self.hovered_monitor().cloned() {
                    let bounds = m.bounds();
                    let image = self.capture_region(bounds);
                    Outcome::Monitor {
                        monitor: m.id(),
                        rect: bounds,
                        image,
                    }
                } else {
                    Outcome::Cancelled
                }
            }
            SelectorMode::Window => {
                if let Some(w) = self.hovered_window().cloned() {
                    let bounds = w.bounds();
                    let image = self.capture_region(bounds);
                    Outcome::Window {
                        window: w.id(),
                        rect: bounds,
                        image,
                    }
                } else {
                    Outcome::Cancelled
                }
            }
            SelectorMode::Area | SelectorMode::AnyOf => match self.canvas.region() {
                Some(r) if r.width() >= 2 && r.height() >= 2 => {
                    let image = self.capture_region(r);
                    Outcome::Region { rect: r, image }
                }
                _ => Outcome::Cancelled,
            },
        };
        self.outcome = Some(outcome);
        self.store_result();
    }

    fn cancel(&mut self) {
        self.outcome = Some(Outcome::Cancelled);
        self.store_result();
    }

    fn store_result(&mut self) {
        let outcome = self.outcome.clone().unwrap_or(Outcome::Cancelled);
        *self.result_slot.lock().unwrap() = Some(Selection {
            outcome,
            canvas: self.canvas.clone(),
            action: self.action.clone(),
        });
    }

    fn capture_region(&self, rect: CapRect) -> Option<CapImage> {
        let raw = match self.initial.as_deref() {
            Some(img) => {
                let local_x = (rect.x() - self.monitors_bb.x()).max(0) as u32;
                let local_y = (rect.y() - self.monitors_bb.y()).max(0) as u32;
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

// ─── Per-window root view ───────────────────────────────────────────────

struct OverlayView {
    shared: Entity<SharedState>,
    monitor: Monitor,
    focus_handle: FocusHandle,
    blur_cache: Rc<RefCell<BlurCache>>,
    _sub: gpui::Subscription,
}

impl OverlayView {
    fn new(
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
        shared: Entity<SharedState>,
        monitor: Monitor,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let sub = cx.observe(&shared, |_, _, cx| cx.notify());
        Self {
            shared,
            monitor,
            focus_handle,
            blur_cache: Rc::new(RefCell::new(BlurCache::default())),
            _sub: sub,
        }
    }
}

impl Focusable for OverlayView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for OverlayView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let shared = self.shared.clone();
        let shared_for_paint = self.shared.clone();
        let monitor = self.monitor.clone();
        let origin = (monitor.bounds().x(), monitor.bounds().y());

        let (
            show_toolbar,
            runtime_mode,
            active_tool,
            palette,
            ui_cfg,
            color_palette,
            current_color,
            current_width,
        ) = {
            let state = self.shared.read(cx);
            (
                state.config.toolbar,
                state.runtime_mode,
                state.canvas.active_tool.kind(),
                state.config.palette.kinds(),
                Arc::new(state.config.ui.clone()),
                state.config.palette.color_palette.clone(),
                state.canvas.active_tool.current_color(),
                state.canvas.active_tool.current_width(),
            )
        };

        // Mouse events arrive in logical pixels (DIPs) relative to the
        // window origin; the canvas state lives in physical pixels (so it
        // round-trips with `sss_capture::Rect`). We multiply by the
        // window's scale_factor at event time so the same code path works
        // on HiDPI outputs.
        let monitor_origin = (
            monitor.bounds().x() as f32,
            monitor.bounds().y() as f32,
        );
        let translate = move |pos: Point<Pixels>, scale: f32| -> FPoint {
            FPoint::new(
                pos.x.as_f32() * scale + monitor_origin.0,
                pos.y.as_f32() * scale + monitor_origin.1,
            )
        };

        let cursor_kind = active_tool;

        div()
            .id("sss-overlay")
            .key_context("SssOverlay")
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .map(|this| apply_tool_cursor(this, cursor_kind))
            .on_mouse_down(MouseButton::Left, {
                let shared = shared.clone();
                move |ev: &MouseDownEvent, window, cx| {
                    let pos = translate(ev.position, window.scale_factor());
                    let should_quit = shared.update(cx, |s, cx| {
                        s.last_cursor = pos;
                        if matches!(s.canvas.active_tool, crate::tool::Tool::Pipette) {
                            if let Some(c) = s.sample_color_at(pos) {
                                let mut prev = s
                                    .tool_before_pipette
                                    .take()
                                    .unwrap_or(crate::tool::Tool::Pointer);
                                prev.apply_color(c);
                                s.canvas.set_tool(prev);
                            } else if let Some(prev) = s.tool_before_pipette.take() {
                                s.canvas.set_tool(prev);
                            }
                            cx.notify();
                            return false;
                        }
                        match s.runtime_mode {
                            SelectorMode::Monitor | SelectorMode::Window => {
                                s.confirm();
                                cx.notify();
                                true
                            }
                            _ => {
                                s.handle_canvas(CanvasEvent::PointerDown(pos));
                                cx.notify();
                                false
                            }
                        }
                    });
                    if should_quit {
                        cx.quit();
                    }
                }
            })
            .on_mouse_move({
                let shared = shared.clone();
                move |ev: &MouseMoveEvent, window, cx| {
                    let pos = translate(ev.position, window.scale_factor());
                    shared.update(cx, |s, cx| {
                        s.last_cursor = pos;
                        s.handle_canvas(CanvasEvent::PointerMove(pos));
                        cx.notify();
                    });
                }
            })
            .on_mouse_up(MouseButton::Left, {
                let shared = shared.clone();
                move |ev: &MouseUpEvent, window, cx| {
                    let pos = translate(ev.position, window.scale_factor());
                    shared.update(cx, |s, cx| {
                        s.last_cursor = pos;
                        s.handle_canvas(CanvasEvent::PointerUp(pos));
                        cx.notify();
                    });
                }
            })
            .on_key_down({
                let shared = shared.clone();
                move |ev: &KeyDownEvent, _, cx| {
                    let key = ev.keystroke.key.as_str();
                    let modifiers = &ev.keystroke.modifiers;
                    let ctrl = modifiers.control;
                    let shift = modifiers.shift;
                    let confirm_with_enter = shared.read(cx).config.confirm_with_enter;
                    let should_quit = shared.update(cx, |s, cx| {
                        let typing = s.canvas.pending_text().is_some();
                        let picking =
                            matches!(s.canvas.active_tool, crate::tool::Tool::Pipette);
                        let mut quit = false;
                        match key {
                            "escape" if typing => {
                                s.handle_canvas(CanvasEvent::TextCancel);
                            }
                            "escape" if picking => {
                                if let Some(prev) = s.tool_before_pipette.take() {
                                    s.canvas.set_tool(prev);
                                }
                            }
                            "escape" => {
                                s.cancel();
                                quit = true;
                            }
                            "enter" if typing => {
                                s.handle_canvas(CanvasEvent::TextCommit);
                            }
                            "enter" if confirm_with_enter => {
                                s.confirm();
                                quit = true;
                            }
                            "backspace" => s.handle_canvas(CanvasEvent::TextBackspace),
                            "delete" => s.handle_canvas(CanvasEvent::Delete),
                            "z" if ctrl && shift => s.handle_canvas(CanvasEvent::Redo),
                            "z" if ctrl => s.handle_canvas(CanvasEvent::Undo),
                            "y" if ctrl => s.handle_canvas(CanvasEvent::Redo),
                            "c" if ctrl => {
                                s.action.copy = true;
                                s.confirm();
                                quit = true;
                            }
                            "s" if ctrl => {
                                s.action.save = true;
                                s.confirm();
                                quit = true;
                            }
                            "space" => {
                                s.handle_canvas(CanvasEvent::TextInput(' '));
                            }
                            single if single.chars().count() == 1 && !ctrl => {
                                if let Some(ch) = single.chars().next() {
                                    s.handle_canvas(CanvasEvent::TextInput(ch));
                                }
                            }
                            _ => {}
                        }
                        cx.notify();
                        quit
                    });
                    if should_quit {
                        cx.quit();
                    }
                }
            })
            .when(show_toolbar, |this| {
                this.child(render_toolbar(
                    shared.clone(),
                    runtime_mode,
                    active_tool,
                    palette,
                    ui_cfg,
                    color_palette,
                    current_color,
                    current_width,
                ))
            })
            .child({
                let monitor_bounds = monitor.bounds();
                let blur_cache = self.blur_cache.clone();
                gpui::canvas(
                    move |_, _, _| {},
                    move |_, _, window, cx| {
                        let scale = window.scale_factor().max(0.0001);
                        let xf = Xform {
                            origin,
                            inv_scale: 1.0 / scale,
                        };
                        let (snapshot, initial, initial_origin, hover) = {
                            let state = shared_for_paint.read(cx);
                            let initial_origin =
                                (state.monitors_bb.x(), state.monitors_bb.y());
                            let hover = match state.runtime_mode {
                                SelectorMode::Monitor => state
                                    .hovered_monitor()
                                    .map(|m| (m.bounds(), None::<String>)),
                                SelectorMode::Window => state
                                    .hovered_window()
                                    .map(|w| (w.bounds(), Some(window_label(w)))),
                                _ => None,
                            };
                            (
                                state.canvas.clone(),
                                state.initial.clone(),
                                initial_origin,
                                hover,
                            )
                        };
                        // 1. Slurp-style dim outside the region (or the whole
                        //    monitor when nothing's selected yet).
                        paint_outside_mask(window, monitor_bounds, snapshot.region(), xf);
                        // 2. Blurred underlays for BlurRect shapes (cached).
                        let mut cache = blur_cache.borrow_mut();
                        paint_blurs(
                            window,
                            &snapshot,
                            initial.as_deref(),
                            initial_origin,
                            xf,
                            &mut cache,
                        );
                        // 3. Shapes + rubber-band outline.
                        paint_canvas(window, cx, &snapshot, xf);
                        if let Some((rect, label)) = hover {
                            paint_hover_target(
                                window,
                                cx,
                                rect,
                                label.as_deref(),
                                monitor_bounds,
                                xf,
                            );
                        }
                        let region = snapshot.region();
                        paint_confirm_hint(window, cx, monitor_bounds, region, xf);
                    },
                )
                .size_full()
            })
    }
}

// ─── Toolbar ────────────────────────────────────────────────────────────

fn render_toolbar(
    shared: Entity<SharedState>,
    active_mode: SelectorMode,
    active_tool: crate::config::ToolKind,
    palette: Vec<crate::config::ToolKind>,
    ui: Arc<crate::config::UiConfig>,
    color_palette: Vec<crate::color::Color>,
    current_color: Option<crate::color::Color>,
    current_width: Option<f32>,
) -> impl IntoElement {
    let bar_bg = hsla(0.0, 0.0, 0.10, 0.92);
    let bar_border = hsla(0.58, 0.7, 0.5, 1.0);

    let mode_btn =
        |id: &'static str, icon: &'static str, mode: SelectorMode, shared: Entity<SharedState>| {
            let selected = mode == active_mode;
            toolbar_icon_button(id, icon, selected, false, move |cx| {
                shared.update(cx, |s, cx| {
                    s.runtime_mode = mode;
                    cx.notify();
                });
            })
        };

    let mut tool_row = div().flex().flex_row().gap_1();
    for kind in palette {
        let selected = kind == active_tool;
        let shared = shared.clone();
        let ui = ui.clone();
        let id: gpui::SharedString = format!("tool-{}", kind.label()).into();
        let icon: gpui::SharedString = kind.icon_path().into();
        tool_row = tool_row.child(toolbar_icon_button(
            id,
            icon,
            selected,
            false,
            move |cx| {
                let built = kind.build(&ui);
                shared.update(cx, |s, cx| {
                    // Committing a half-typed Text shape so switching
                    // tools doesn't leave it stuck in the editor.
                    if matches!(&s.canvas.active_tool, crate::tool::Tool::Text(_)) {
                        s.handle_canvas(CanvasEvent::TextCommit);
                    }
                    if matches!(built, crate::tool::Tool::Pipette) {
                        // Remember where to return after the sample. Don't
                        // overwrite a previous stash if the user mashes the
                        // button twice without sampling.
                        if !matches!(s.canvas.active_tool, crate::tool::Tool::Pipette) {
                            s.tool_before_pipette = Some(s.canvas.active_tool.clone());
                        }
                    } else {
                        s.tool_before_pipette = None;
                    }
                    s.canvas.set_tool(built);
                    cx.notify();
                });
            },
        ));
    }

    div()
        .id("sss-toolbar")
        .absolute()
        .top(px(12.))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        // Swallow every pointer event over the bar so clicks on the
        // tool buttons don't bubble into the canvas hitbox and start a
        // stray region drag underneath.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex()
                .flex_row()
                .gap_3()
                .items_center()
                .px_3()
                .py_2()
                .rounded_lg()
                .bg(bar_bg)
                .border_1()
                .border_color(bar_border)
                .text_color(white())
                .text_size(px(13.))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_1()
                        .child(mode_btn(
                            "mode-area",
                            "icons/area.svg",
                            SelectorMode::Area,
                            shared.clone(),
                        ))
                        .child(mode_btn(
                            "mode-monitor",
                            "icons/monitor.svg",
                            SelectorMode::Monitor,
                            shared.clone(),
                        ))
                        .child(mode_btn(
                            "mode-window",
                            "icons/window.svg",
                            SelectorMode::Window,
                            shared.clone(),
                        )),
                )
                .child(toolbar_divider())
                .child(tool_row)
                .when(current_color.is_some() && !color_palette.is_empty(), |this| {
                    this.child(toolbar_divider())
                        .child(render_color_row(
                            shared.clone(),
                            color_palette.clone(),
                            current_color,
                        ))
                })
                .when(current_width.is_some(), |this| {
                    this.child(toolbar_divider())
                        .child(render_width_controls(
                            shared.clone(),
                            current_width.unwrap(),
                        ))
                })
                .child(toolbar_divider())
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_1()
                        .child(toolbar_icon_button(
                            "action-undo",
                            "icons/undo.svg",
                            false,
                            false,
                            {
                                let shared = shared.clone();
                                move |cx| {
                                    shared.update(cx, |s, cx| {
                                        s.handle_canvas(CanvasEvent::Undo);
                                        cx.notify();
                                    });
                                }
                            },
                        ))
                        .child(toolbar_icon_button(
                            "action-redo",
                            "icons/redo.svg",
                            false,
                            false,
                            {
                                let shared = shared.clone();
                                move |cx| {
                                    shared.update(cx, |s, cx| {
                                        s.handle_canvas(CanvasEvent::Redo);
                                        cx.notify();
                                    });
                                }
                            },
                        )),
                )
                .child(toolbar_divider())
                .child(toolbar_icon_button(
                    "action-cancel",
                    "icons/cancel.svg",
                    false,
                    false,
                    {
                        let shared = shared.clone();
                        move |cx| {
                            shared.update(cx, |s, _| s.cancel());
                            cx.quit();
                        }
                    },
                ))
                .child(toolbar_icon_button(
                    "action-confirm",
                    "icons/confirm.svg",
                    false,
                    true,
                    {
                        let shared = shared.clone();
                        move |cx| {
                            shared.update(cx, |s, _| s.confirm());
                            cx.quit();
                        }
                    },
                )),
        )
}

fn toolbar_divider() -> impl IntoElement {
    div().w(px(1.)).h(px(20.)).bg(hsla(0., 0., 1., 0.18))
}

fn apply_tool_cursor(
    elem: gpui::Stateful<gpui::Div>,
    tool: crate::config::ToolKind,
) -> gpui::Stateful<gpui::Div> {
    use crate::config::ToolKind;
    match tool {
        ToolKind::Pointer => elem.cursor_default(),
        ToolKind::Pipette => elem.cursor_pointer(),
        ToolKind::Text => elem.cursor_text(),
        ToolKind::Eraser => elem.cursor(gpui::CursorStyle::DragCopy),
        _ => elem.cursor_crosshair(),
    }
}

fn window_label(w: &sss_capture::Window) -> String {
    let app = w.app_name();
    let title = w.title();
    match (app.is_empty(), title.is_empty()) {
        (false, false) => format!("{app} — {title}"),
        (false, true) => app.into(),
        (true, false) => title.into(),
        (true, true) => format!("Window {}", w.id()),
    }
}

fn render_color_row(
    shared: Entity<SharedState>,
    palette: Vec<crate::color::Color>,
    current: Option<crate::color::Color>,
) -> impl IntoElement {
    let mut row = div().flex().flex_row().gap_1().items_center();
    for color in palette {
        let selected = current == Some(color);
        let shared_clone = shared.clone();
        row = row.child(color_swatch(color, selected, move |cx| {
            shared_clone.update(cx, |s, cx| {
                s.canvas.active_tool.apply_color(color);
                cx.notify();
            });
        }));
    }
    row
}

fn color_swatch(
    color: crate::color::Color,
    selected: bool,
    on_click: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    let [r, g, b, a] = color.0;
    let bg = gpui::Rgba {
        r: r as f32 / 255.,
        g: g as f32 / 255.,
        b: b as f32 / 255.,
        a: a as f32 / 255.,
    };
    let id: gpui::SharedString = format!("swatch-{r}-{g}-{b}-{a}").into();
    let ring_color = if selected {
        hsla(0.58, 0.7, 0.7, 1.0)
    } else {
        hsla(0., 0., 1., 0.25)
    };
    div()
        .id(gpui::ElementId::Name(id))
        .w(px(22.))
        .h(px(22.))
        .rounded_full()
        .bg(bg)
        .border_2()
        .border_color(ring_color)
        .hover(|s| s.opacity(0.85))
        .cursor_pointer()
        .on_click(move |_, _, cx| on_click(cx))
}

fn render_width_controls(
    shared: Entity<SharedState>,
    current: f32,
) -> impl IntoElement {
    let step = if current < 4.0 {
        0.5
    } else if current < 12.0 {
        1.0
    } else {
        2.0
    };
    let value = if current.fract().abs() < 0.05 {
        format!("{:.0}", current)
    } else {
        format!("{:.1}", current)
    };

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .child(toolbar_text_button("width-dec", "−", false, {
            let shared = shared.clone();
            move |cx| {
                shared.update(cx, |s, cx| {
                    if let Some(w) = s.canvas.active_tool.current_width() {
                        s.canvas.active_tool.apply_width((w - step).max(0.5));
                        cx.notify();
                    }
                });
            }
        }))
        .child(
            div()
                .w(px(36.))
                .text_color(white())
                .text_size(px(12.))
                .child(value),
        )
        .child(toolbar_text_button("width-inc", "+", false, {
            let shared = shared.clone();
            move |cx| {
                shared.update(cx, |s, cx| {
                    if let Some(w) = s.canvas.active_tool.current_width() {
                        s.canvas.active_tool.apply_width((w + step).min(120.0));
                        cx.notify();
                    }
                });
            }
        }))
}

fn toolbar_text_button(
    id: &'static str,
    label: &'static str,
    primary: bool,
    on_click: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    let bg = if primary {
        hsla(0.36, 0.7, 0.45, 1.0)
    } else {
        hsla(0.0, 0.0, 0.20, 1.0)
    };
    div()
        .id(gpui::ElementId::Name(id.into()))
        .w(px(24.))
        .h(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .bg(bg)
        .text_color(white())
        .text_size(px(14.))
        .hover(|s| s.opacity(0.85))
        .active(|s| s.opacity(0.7))
        .cursor_pointer()
        .child(label)
        .on_click(move |_, _, cx| on_click(cx))
}

fn toolbar_icon_button(
    id: impl Into<gpui::SharedString>,
    icon: impl Into<gpui::SharedString>,
    selected: bool,
    primary: bool,
    on_click: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    let (bg, fg) = if selected {
        (hsla(0.58, 0.7, 0.4, 1.0), white())
    } else if primary {
        (hsla(0.36, 0.7, 0.45, 1.0), white())
    } else {
        (hsla(0.0, 0.0, 0.20, 1.0), hsla(0.0, 0.0, 0.9, 1.0))
    };
    div()
        .id(gpui::ElementId::Name(id.into()))
        .w(px(32.))
        .h(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .bg(bg)
        .text_color(fg)
        .hover(|s| s.opacity(0.85))
        .active(|s| s.opacity(0.7))
        .cursor_pointer()
        .child(
            gpui::svg()
                .path(icon)
                .size(px(18.))
                .text_color(fg),
        )
        .on_click(move |_, _, cx| on_click(cx))
}
