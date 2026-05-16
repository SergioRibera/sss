//! egui-based interactive overlay (toolbar and canvas painter).

use egui::{Color32, Pos2, Rect as EguiRect, Stroke, Vec2};

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
    ctx: &mut egui::Ui,
    canvas: &mut Canvas,
    palette: &ToolPalette,
    mode: &mut SelectorMode,
    cfg: ToolbarConfig,
) -> ToolbarOutput {
    let mut out = ToolbarOutput::default();
    egui::Panel::top("sss_capture_ui::toolbar")
        .resizable(false)
        .show_inside(ctx, |ui| {
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
pub fn draw_canvas(painter: &egui::Painter, canvas: &Canvas, screen_offset: Pos2) {
    if let Some(rect) = canvas.region() {
        let r = EguiRect::from_min_size(
            Pos2::new(
                rect.x() as f32 - screen_offset.x,
                rect.y() as f32 - screen_offset.y,
            ),
            Vec2::new(rect.width() as f32, rect.height() as f32),
        );
        painter.rect_stroke(
            r,
            0.0,
            Stroke::new(2.0, Color32::from_rgb(90, 170, 255)),
            egui::StrokeKind::Middle,
        );
    }
    for shape in canvas.shapes() {
        draw_shape(painter, shape, screen_offset);
    }
    if let Some(preview) = canvas.preview_shape() {
        draw_shape(painter, &preview, screen_offset);
    }
    if let Some(pending) = canvas.pending_text() {
        draw_shape(painter, &pending, screen_offset);
    }
}

fn draw_shape(painter: &egui::Painter, shape: &Shape, off: Pos2) {
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
        ShapeKind::Rectangle { rect } | ShapeKind::BlurRect { rect, .. } => {
            let r = EguiRect::from_min_size(
                Pos2::new(rect.x() as f32 - off.x, rect.y() as f32 - off.y),
                Vec2::new(rect.width() as f32, rect.height() as f32),
            );
            if let Some(f) = fill {
                painter.rect_filled(r, 0.0, f);
            }
            painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Middle);
        }
        ShapeKind::Ellipse { rect } => {
            let r = EguiRect::from_min_size(
                Pos2::new(rect.x() as f32 - off.x, rect.y() as f32 - off.y),
                Vec2::new(rect.width() as f32, rect.height() as f32),
            );
            painter.add(egui::Shape::ellipse_stroke(
                r.center(),
                r.size() / 2.0,
                stroke,
            ));
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
