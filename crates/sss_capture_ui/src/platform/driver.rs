//! GPUI-backed driver: one root view per output, with a shared canvas
//! state observed by every window. On Linux Wayland the windows are
//! `WindowKind::LayerShell` surfaces; everywhere else they fall back to a
//! borderless fullscreen window.

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
use crate::geometry::FPoint;
use crate::mode::SelectorMode;
use crate::render::overlay::{Xform, paint_canvas, paint_confirm_hint};
use crate::selector::{Config, Outcome, PostAction, Selection, Selector, SelectorError};

// ─── Public entry point ─────────────────────────────────────────────────

pub fn run(sel: Selector) -> Result<Selection, SelectorError> {
    let Selector { config, capturer } = sel;

    let initial = match capturer.capture_all_with(config.capture_opts) {
        Ok(img) => Some(img),
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

    let app: Application = gpui_platform::application();
    {
        let monitors = monitors.clone();
        let result_slot = result_slot.clone();
        app.run(move |cx| {
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
                monitors_bb,
                result_slot,
            });

            // Match each sss_capture monitor to a GPUI display by origin.
            // On Wayland the display_id pins the layer-shell surface to the
            // right output; on macOS/X11 it picks the fullscreen target.
            let displays = cx.displays();
            for monitor in monitors.iter() {
                let m_bounds = monitor.bounds();
                let display_id = displays
                    .iter()
                    .find(|d| {
                        let b = d.bounds();
                        (b.origin.x.as_f32() as i32 - m_bounds.x()).abs() < 4
                            && (b.origin.y.as_f32() as i32 - m_bounds.y()).abs() < 4
                    })
                    .map(|d| d.id());

                let opts = window_options_for(monitor, display_id);
                let shared_for_window = shared.clone();
                let monitor_clone = monitor.clone();
                let _ = cx.open_window(opts, move |window, cx| {
                    cx.new(|cx| OverlayView::new(window, cx, shared_for_window, monitor_clone))
                });
            }

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

fn window_options_for(monitor: &Monitor, display_id: Option<gpui::DisplayId>) -> WindowOptions {
    let m = monitor.bounds();
    let bounds = Bounds {
        origin: point(px(m.x() as f32), px(m.y() as f32)),
        size: size(px(m.width() as f32), px(m.height() as f32)),
    };

    let kind = layer_shell_kind();

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
    initial: Option<CapImage>,
    monitors_bb: CapRect,
    result_slot: Arc<Mutex<Option<Selection>>>,
}

impl SharedState {
    fn handle_canvas(&mut self, ev: CanvasEvent) {
        self.canvas.handle(ev);
    }

    fn confirm(&mut self) {
        let outcome = match self.runtime_mode {
            SelectorMode::Monitor => {
                let p = sss_capture::Point::new(
                    self.last_cursor.x as i32,
                    self.last_cursor.y as i32,
                );
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
                let cursor = sss_capture::Point::new(
                    self.last_cursor.x as i32,
                    self.last_cursor.y as i32,
                );
                let win = self
                    .capturer
                    .windows()
                    .ok()
                    .and_then(|ws| ws.into_iter().find(|w| w.bounds().contains(cursor)));
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
        let raw = match self.initial.as_ref() {
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

        let (show_toolbar, runtime_mode) = {
            let state = self.shared.read(cx);
            (state.config.toolbar, state.runtime_mode)
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

        div()
            .id("sss-overlay")
            .key_context("SssOverlay")
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .cursor_crosshair()
            .bg(hsla(0.0, 0.0, 0.0, 0.18))
            .on_mouse_down(MouseButton::Left, {
                let shared = shared.clone();
                move |ev: &MouseDownEvent, window, cx| {
                    let pos = translate(ev.position, window.scale_factor());
                    shared.update(cx, |s, cx| {
                        s.last_cursor = pos;
                        s.handle_canvas(CanvasEvent::PointerDown(pos));
                        cx.notify();
                    });
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
                        let mut quit = false;
                        match key {
                            "escape" => {
                                s.cancel();
                                quit = true;
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
                this.child(render_toolbar(shared.clone(), runtime_mode))
            })
            .child({
                let monitor_bounds = monitor.bounds();
                gpui::canvas(
                    move |_, _, _| {},
                    move |_, _, window, cx| {
                        let snapshot = shared_for_paint.read(cx).canvas.clone();
                        let xf = Xform::new(origin, window.scale_factor());
                        paint_canvas(window, cx, &snapshot, xf);
                        let region = snapshot.region();
                        paint_confirm_hint(window, cx, monitor_bounds, region, xf);
                    },
                )
                .size_full()
            })
    }
}

// ─── Toolbar ────────────────────────────────────────────────────────────

fn render_toolbar(shared: Entity<SharedState>, active_mode: SelectorMode) -> impl IntoElement {
    let bar_bg = hsla(0.0, 0.0, 0.10, 0.92);
    let bar_border = hsla(0.58, 0.7, 0.5, 1.0);

    let mode_btn = |label: &'static str, mode: SelectorMode, shared: Entity<SharedState>| {
        let selected = mode == active_mode;
        toolbar_button(label.into(), selected, false, move |cx| {
            shared.update(cx, |s, cx| {
                s.runtime_mode = mode;
                cx.notify();
            });
        })
    };

    div()
        .id("sss-toolbar")
        .absolute()
        .top(px(12.))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .px_3()
                .py_2()
                .rounded_lg()
                .bg(bar_bg)
                .border_1()
                .border_color(bar_border)
                .text_color(white())
                .text_size(px(13.))
                .child(mode_btn("Area", SelectorMode::Area, shared.clone()))
                .child(mode_btn("Monitor", SelectorMode::Monitor, shared.clone()))
                .child(mode_btn("Window", SelectorMode::Window, shared.clone()))
                .child(toolbar_button("Undo".into(), false, false, {
                    let shared = shared.clone();
                    move |cx| {
                        shared.update(cx, |s, cx| {
                            s.handle_canvas(CanvasEvent::Undo);
                            cx.notify();
                        });
                    }
                }))
                .child(toolbar_button("Redo".into(), false, false, {
                    let shared = shared.clone();
                    move |cx| {
                        shared.update(cx, |s, cx| {
                            s.handle_canvas(CanvasEvent::Redo);
                            cx.notify();
                        });
                    }
                }))
                .child(toolbar_button("Cancel".into(), false, false, {
                    let shared = shared.clone();
                    move |cx| {
                        shared.update(cx, |s, _| s.cancel());
                        cx.quit();
                    }
                }))
                .child(toolbar_button("Capture".into(), false, true, {
                    let shared = shared.clone();
                    move |cx| {
                        shared.update(cx, |s, _| s.confirm());
                        cx.quit();
                    }
                })),
        )
}

fn toolbar_button(
    label: String,
    selected: bool,
    primary: bool,
    on_click: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    let bg = if selected {
        hsla(0.58, 0.7, 0.4, 1.0)
    } else if primary {
        hsla(0.36, 0.7, 0.45, 1.0)
    } else {
        hsla(0.0, 0.0, 0.20, 1.0)
    };
    div()
        .id(gpui::ElementId::Name(label.clone().into()))
        .px_3()
        .py_1()
        .rounded_md()
        .bg(bg)
        .text_color(white())
        .hover(|s| s.opacity(0.85))
        .active(|s| s.opacity(0.7))
        .cursor_pointer()
        .child(label)
        .on_click(move |_, _, cx| on_click(cx))
}
