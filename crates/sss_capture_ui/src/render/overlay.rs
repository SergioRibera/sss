//! egui-based interactive overlay — canvas painter + confirm hint. The
//! toolbar / popup / radial chrome lives in [`crate::render::ui`].

#![allow(dead_code)]

use egui::{Color32, Pos2, Rect as EguiRect, Stroke, Vec2};
use sss_capture::Rect as CapRect;

use crate::canvas::Canvas;
use crate::color::Color;
use crate::mode::SelectorMode;
use crate::shape::{Shape, ShapeKind};
use crate::tool::{BrushSettings, StepSettings, Tool, ToolPalette};

/// Toolbar request flags; reset to false each frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct ToolbarOutput {
    pub confirm: bool,
    pub copy: bool,
    pub save: bool,
    pub cancel: bool,
}

/// Which action buttons the toolbar should show.
#[derive(Clone, Copy, Debug)]
pub struct ToolbarConfig {
    pub show_copy: bool,
    pub show_save: bool,
}

impl Default for ToolbarConfig {
    fn default() -> Self {
        Self {
            show_copy: true,
            show_save: true,
        }
    }
}

/// Paints the toolbar at the top of the active output.
pub fn draw_toolbar(
    ctx: &egui::Context,
    canvas: &mut Canvas,
    palette: &ToolPalette,
    mode: &mut SelectorMode,
    cfg: ToolbarConfig,
) -> ToolbarOutput {
    let mut out = ToolbarOutput::default();
    egui::TopBottomPanel::top("sss_capture_ui::toolbar")
        .resizable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Mode");
                for m in [
                    SelectorMode::Area,
                    SelectorMode::Monitor,
                    SelectorMode::Window,
                ] {
                    if ui.selectable_label(*mode == m, m.label()).clicked() {
                        *mode = m;
                    }
                }
                ui.separator();

                for tool in &palette.tools {
                    let selected =
                        std::mem::discriminant(&canvas.active_tool) == std::mem::discriminant(tool);
                    if ui
                        .selectable_label(selected, format!("{} {}", tool.icon(), tool.name()))
                        .clicked()
                    {
                        canvas.set_tool(tool.clone());
                    }
                }
                ui.separator();

                for color in &palette.color_palette {
                    let c32 = to_color32(*color);
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::click());
                    ui.painter().rect_filled(rect, 3.0, c32);
                    if resp.clicked() {
                        apply_color(&mut canvas.active_tool, *color);
                    }
                }
                ui.separator();

                if ui.button("Undo").clicked() {
                    canvas.handle(crate::canvas::CanvasEvent::Undo);
                }
                if ui.button("Redo").clicked() {
                    canvas.handle(crate::canvas::CanvasEvent::Redo);
                }
                ui.separator();

                if ui
                    .button("✕  Cancel")
                    .on_hover_text("Discard the selection (Esc)")
                    .clicked()
                {
                    out.cancel = true;
                }
                if ui
                    .button("✓  Capture")
                    .on_hover_text("Finish editing (Enter)")
                    .clicked()
                {
                    out.confirm = true;
                }
                if cfg.show_copy
                    && ui
                        .button("📋 Copy")
                        .on_hover_text("Copy the edited image to the clipboard (Ctrl+C)")
                        .clicked()
                {
                    out.copy = true;
                    out.confirm = true;
                }
                if cfg.show_save
                    && ui
                        .button("💾 Save")
                        .on_hover_text("Save the edited image to disk (Ctrl+S)")
                        .clicked()
                {
                    out.save = true;
                    out.confirm = true;
                }
            });
        });
    out
}

/// Paint the region rubber-band and every shape onto an egui painter.
pub fn draw_canvas(
    painter: &egui::Painter,
    canvas: &Canvas,
    screen_offset: Pos2,
    pointer_global: Option<crate::geometry::FPoint>,
    blurred_bg: Option<&egui::TextureHandle>,
    monitor_size_px: (u32, u32),
    region_color: Color32,
) {
    if let Some(rect) = canvas.region() {
        let r = EguiRect::from_min_size(
            Pos2::new(
                rect.x() as f32 - screen_offset.x,
                rect.y() as f32 - screen_offset.y,
            ),
            Vec2::new(rect.width() as f32, rect.height() as f32),
        );
        let stroke = Stroke::new(1.5, region_color);
        draw_dashed_rect(painter, r, stroke, 8.0, 5.0);
    }
    for shape in canvas.shapes() {
        draw_shape(painter, shape, screen_offset, blurred_bg, monitor_size_px);
    }
    if let Some(preview) = canvas.preview_shape() {
        draw_shape(painter, &preview, screen_offset, blurred_bg, monitor_size_px);
    }
    if let Some(pending) = canvas.pending_text() {
        draw_shape(painter, &pending, screen_offset, blurred_bg, monitor_size_px);
    }
    // Polygon-in-progress preview: mirror what the committed polygon will
    // look like (fill if fill mode is on, closing line back to the first
    // vertex), plus a live guide line from the last vertex to the pointer
    // and vertex markers so the user can fine-tune placement.
    if let Some(verts) = canvas.polygon_vertices() {
        if !verts.is_empty() {
            let style = canvas.current_polygon_style();
            let stroke_col = to_color32(style.stroke);
            let stroke = Stroke::new(style.stroke_width.max(1.0), stroke_col);
            let fill = style.fill.map(to_color32);
            let mut pts: Vec<Pos2> = verts
                .iter()
                .map(|p| Pos2::new(p.x - screen_offset.x, p.y - screen_offset.y))
                .collect();
            // Live guide segment from the last placed vertex to the
            // pointer — translucent so it reads as "next click goes here".
            let live_tip = pointer_global
                .map(|p| Pos2::new(p.x - screen_offset.x, p.y - screen_offset.y));
            // Closing-segment preview back to the first vertex (dashed).
            let closing_stroke = Stroke::new(
                stroke.width.max(1.0),
                stroke_col.gamma_multiply(0.55),
            );

            // Fill preview: convex polygon of placed vertices + live tip
            // so it grows with the cursor.
            if let Some(fill_col) = fill {
                let mut poly = pts.clone();
                if let Some(tip) = live_tip {
                    poly.push(tip);
                }
                if poly.len() >= 3 {
                    painter.add(egui::Shape::convex_polygon(
                        poly,
                        fill_col,
                        Stroke::NONE,
                    ));
                }
            }

            // Solid stroke through placed vertices.
            if pts.len() >= 2 {
                painter.add(egui::Shape::line(pts.clone(), stroke));
            }
            // Guide from last vertex to pointer.
            if let (Some(last), Some(tip)) = (pts.last().copied(), live_tip) {
                painter.line_segment(
                    [last, tip],
                    Stroke::new(stroke.width, stroke_col.gamma_multiply(0.7)),
                );
            }
            // Dashed close hint back to first vertex.
            if pts.len() >= 2 {
                let first = pts[0];
                let last = *pts.last().unwrap();
                draw_dashed_segment(painter, last, first, closing_stroke, 6.0, 4.0);
            }
            // Vertex markers.
            pts.extend(live_tip);
            for (i, p) in pts.iter().enumerate() {
                let r = if i == 0 { 4.5 } else { 3.5 };
                painter.circle_filled(*p, r, stroke_col);
                painter.circle_stroke(*p, r + 0.5, Stroke::new(1.0, Color32::WHITE));
            }
        }
    }
}

fn draw_dashed_segment(
    painter: &egui::Painter,
    a: Pos2,
    b: Pos2,
    stroke: Stroke,
    dash_on: f32,
    dash_off: f32,
) {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / len;
    let uy = dy / len;
    let step = dash_on + dash_off;
    let mut t = 0.0;
    while t < len {
        let t1 = (t + dash_on).min(len);
        painter.line_segment(
            [
                Pos2::new(a.x + ux * t, a.y + uy * t),
                Pos2::new(a.x + ux * t1, a.y + uy * t1),
            ],
            stroke,
        );
        t += step;
    }
}

/// Paint the "Press Enter to accept" hint below the active region (or at the
/// bottom of the monitor when there is none). `screen_rect` is the panel rect
/// in egui coords, `monitor_origin` is the monitor's global origin in the same
/// physical-pixel space the canvas uses, and `monitor_width` is the monitor's
/// width in that space.
pub fn draw_confirm_hint(
    painter: &egui::Painter,
    screen_rect: EguiRect,
    region: Option<CapRect>,
    monitor_origin: Pos2,
    monitor_width: f32,
    hint: &str,
    chrome: &crate::config::ChromeColors,
) {
    let text = hint;
    let font_id = egui::FontId::proportional(16.0);
    let text_color = Color32::from_rgb(
        chrome.toolbar_fg.0[0],
        chrome.toolbar_fg.0[1],
        chrome.toolbar_fg.0[2],
    );
    let galley = painter.layout_no_wrap(text.to_owned(), font_id, text_color);
    let pad = Vec2::new(14.0, 8.0);
    let panel_size = galley.size() + pad * 2.0;

    let panel_pos = if let Some(region) = region.filter(|r| r.width() >= 2 && r.height() >= 2) {
        // Single label across multi-monitor regions: only the monitor whose
        // horizontal span covers the region centre draws the hint.
        let center_gx = (region.x() + region.width() as i32 / 2) as f32;
        if center_gx < monitor_origin.x || center_gx >= monitor_origin.x + monitor_width {
            return;
        }
        let local_cx = center_gx - monitor_origin.x;
        let top_local = region.y() as f32 - monitor_origin.y;
        let bottom_local = top_local + region.height() as f32;
        let margin = 16.0;
        let below_y = bottom_local + margin;
        let panel_y = if below_y + panel_size.y <= screen_rect.height() - 8.0 {
            below_y
        } else if top_local - panel_size.y - margin >= 8.0 {
            top_local - panel_size.y - margin
        } else {
            (screen_rect.height() - panel_size.y - 8.0).max(8.0)
        };
        let max_x = (screen_rect.width() - panel_size.x - 8.0).max(8.0);
        let panel_x = (local_cx - panel_size.x / 2.0).clamp(8.0, max_x);
        Pos2::new(panel_x, panel_y)
    } else {
        let bottom_margin = 48.0;
        Pos2::new(
            ((screen_rect.width() - panel_size.x) / 2.0).max(8.0),
            (screen_rect.height() - panel_size.y - bottom_margin).max(8.0),
        )
    };

    let panel_rect = EguiRect::from_min_size(panel_pos, panel_size);
    let panel_bg = Color32::from_rgb(
        chrome.toolbar_bg.0[0],
        chrome.toolbar_bg.0[1],
        chrome.toolbar_bg.0[2],
    );
    let panel_border = Color32::from_rgb(
        chrome.accent.0[0],
        chrome.accent.0[1],
        chrome.accent.0[2],
    );
    painter.rect_filled(panel_rect, 0.0, panel_bg);
    painter.rect_stroke(
        panel_rect,
        0.0,
        Stroke::new(1.0, panel_border),
        egui::StrokeKind::Middle,
    );
    painter.galley(panel_pos + pad, galley, text_color);
}

fn draw_shape(
    painter: &egui::Painter,
    shape: &Shape,
    off: Pos2,
    blurred_bg: Option<&egui::TextureHandle>,
    monitor_size_px: (u32, u32),
) {
    let stroke = Stroke::new(shape.style.stroke_width, to_color32(shape.style.stroke));
    let fill = shape.style.fill.map(to_color32);
    match &shape.kind {
        ShapeKind::FreehandStroke { points } => {
            let pts: Vec<Pos2> = points
                .iter()
                .map(|p| Pos2::new(p.x - off.x, p.y - off.y))
                .collect();
            painter.add(egui::Shape::line(pts, stroke));
        }
        ShapeKind::Line { from, to } => {
            painter.line_segment(
                [
                    Pos2::new(from.x - off.x, from.y - off.y),
                    Pos2::new(to.x - off.x, to.y - off.y),
                ],
                stroke,
            );
        }
        ShapeKind::Arrow { from, to } => {
            let a = Pos2::new(from.x - off.x, from.y - off.y);
            let b = Pos2::new(to.x - off.x, to.y - off.y);
            painter.line_segment([a, b], stroke);
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let ux = dx / len;
            let uy = dy / len;
            let head = (stroke.width * 3.0).max(10.0);
            let p1 = Pos2::new(
                b.x - (ux * head + uy * head * 0.5),
                b.y - (uy * head - ux * head * 0.5),
            );
            let p2 = Pos2::new(
                b.x - (ux * head - uy * head * 0.5),
                b.y - (uy * head + ux * head * 0.5),
            );
            painter.line_segment([b, p1], stroke);
            painter.line_segment([b, p2], stroke);
        }
        ShapeKind::Rectangle { rect } => {
            let r = EguiRect::from_min_size(
                Pos2::new(rect.x() as f32 - off.x, rect.y() as f32 - off.y),
                Vec2::new(rect.width() as f32, rect.height() as f32),
            );
            if let Some(f) = fill {
                painter.rect_filled(r, 0.0, f);
            }
            painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Middle);
        }
        ShapeKind::BlurRect { rect, .. } => {
            // Live preview: blit the pre-blurred bg slice into the rect so
            // the user sees the actual blur applied (not a fake overlay).
            // The shape's per-instance radius is honoured at composite
            // time on the final image.
            let r = EguiRect::from_min_size(
                Pos2::new(rect.x() as f32 - off.x, rect.y() as f32 - off.y),
                Vec2::new(rect.width() as f32, rect.height() as f32),
            );
            if let (Some(tex), (mw, mh)) = (blurred_bg, monitor_size_px) {
                if mw > 0 && mh > 0 {
                    // UV sampling from this monitor's pre-blurred copy.
                    let u0 = ((rect.x() as f32 - off.x) / mw as f32).clamp(0.0, 1.0);
                    let v0 = ((rect.y() as f32 - off.y) / mh as f32).clamp(0.0, 1.0);
                    let u1 = ((rect.x() as f32 + rect.width() as f32 - off.x) / mw as f32)
                        .clamp(0.0, 1.0);
                    let v1 = ((rect.y() as f32 + rect.height() as f32 - off.y) / mh as f32)
                        .clamp(0.0, 1.0);
                    painter.image(
                        tex.id(),
                        r,
                        EguiRect::from_min_max(Pos2::new(u0, v0), Pos2::new(u1, v1)),
                        Color32::WHITE,
                    );
                }
            } else {
                painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(180, 200, 230, 70));
            }
            let dash_stroke = Stroke::new(1.0, Color32::from_rgb(200, 220, 255));
            draw_dashed_rect(painter, r, dash_stroke, 6.0, 4.0);
        }
        ShapeKind::Ellipse { rect } => {
            let r = EguiRect::from_min_size(
                Pos2::new(rect.x() as f32 - off.x, rect.y() as f32 - off.y),
                Vec2::new(rect.width() as f32, rect.height() as f32),
            );
            painter.add(egui::Shape::Ellipse(egui::epaint::EllipseShape {
                center: r.center(),
                radius: r.size() / 2.0,
                fill: fill.unwrap_or(Color32::TRANSPARENT),
                stroke: stroke.into(),
            }));
        }
        ShapeKind::Step {
            center,
            number,
            radius,
        } => {
            let c = Pos2::new(center.x - off.x, center.y - off.y);
            painter.circle_filled(c, *radius, fill.unwrap_or(stroke.color));
            painter.circle_stroke(c, *radius, Stroke::new(1.0, Color32::WHITE));
            painter.text(
                c,
                egui::Align2::CENTER_CENTER,
                number.to_string(),
                egui::FontId::proportional(radius * 1.1),
                Color32::WHITE,
            );
        }
        ShapeKind::Text {
            origin,
            content,
            style,
        } => {
            painter.text(
                Pos2::new(origin.x - off.x, origin.y - off.y),
                egui::Align2::LEFT_TOP,
                content,
                egui::FontId::proportional(style.size),
                to_color32(style.color),
            );
        }
        ShapeKind::Polygon { points, closed } => {
            if points.is_empty() {
                return;
            }
            let pts: Vec<Pos2> = points
                .iter()
                .map(|p| Pos2::new(p.x - off.x, p.y - off.y))
                .collect();
            if let Some(fill) = fill {
                if *closed && pts.len() >= 3 {
                    painter.add(egui::Shape::convex_polygon(pts.clone(), fill, stroke));
                    return;
                }
            }
            painter.add(egui::Shape::line(pts, stroke));
            if *closed && points.len() >= 3 {
                let first = Pos2::new(points[0].x - off.x, points[0].y - off.y);
                let last = points[points.len() - 1];
                let last = Pos2::new(last.x - off.x, last.y - off.y);
                painter.line_segment([last, first], stroke);
            }
        }
    }
}

fn draw_dashed_rect(
    painter: &egui::Painter,
    rect: EguiRect,
    stroke: Stroke,
    dash_on: f32,
    dash_off: f32,
) {
    let segs = [
        (rect.min, Pos2::new(rect.max.x, rect.min.y)),
        (Pos2::new(rect.max.x, rect.min.y), rect.max),
        (rect.max, Pos2::new(rect.min.x, rect.max.y)),
        (Pos2::new(rect.min.x, rect.max.y), rect.min),
    ];
    for (a, b) in segs {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let ux = dx / len;
        let uy = dy / len;
        let step = dash_on + dash_off;
        let mut t = 0.0;
        while t < len {
            let t1 = (t + dash_on).min(len);
            painter.line_segment(
                [
                    Pos2::new(a.x + ux * t, a.y + uy * t),
                    Pos2::new(a.x + ux * t1, a.y + uy * t1),
                ],
                stroke,
            );
            t += step;
        }
    }
}

fn to_color32(c: Color) -> Color32 {
    let [r, g, b, a] = c.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn apply_color(tool: &mut Tool, color: Color) {
    match tool {
        Tool::Brush(b)
        | Tool::Line(b)
        | Tool::Arrow(b)
        | Tool::Rectangle(b)
        | Tool::Ellipse(b)
        | Tool::Polygon(b) => b.color = color,
        Tool::Step(s) => s.fill = color,
        Tool::Text(t) => t.color = color,
        Tool::Pointer | Tool::Eraser { .. } | Tool::BlurRect { .. } => {}
    }
}

#[allow(dead_code)]
fn _silence_settings(_: BrushSettings, _: StepSettings) {}
