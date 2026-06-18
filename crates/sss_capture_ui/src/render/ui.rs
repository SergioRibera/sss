//! Egui-side overlay chrome: floating icon-only toolbar, radial colour /
//! width menu, popups, magnifier, gizmos.
//!
//! ## Layout overview
//!
//! - Main toolbar is a rounded chip anchored above / below the active
//!   region. Carries tools | colour swatch + width chip | pipette / snap /
//!   magnifier toggles.
//! - Side action toolbar pinned next to the region: undo / redo / clear /
//!   confirm / copy / save / cancel.
//! - Selection toolbar near the bounds of the selected shape: raise /
//!   lower / trash.
//! - Radial menu opens on right-click — 4-column colour grid + width row.
//! - HSV color picker popup opens on swatch click — SV quad + hue strip +
//!   editable hex input.

use std::collections::HashMap;

use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2};

use crate::canvas::Canvas;
use crate::color::Color as SssColor;
use crate::config::ChromeColors;
use crate::icons::{
    filled_tool_icon, rasterise as rasterise_icon, set_active_tool_width, tool_icon, ToolbarIcon,
};
use crate::mode::SelectorMode;
use crate::tool::{Tool, ToolPalette};

const TB_BTN: f32 = 26.0;
const TB_GAP: f32 = 3.0;
const TB_SEP: f32 = 10.0;
const TB_PAD_X: f32 = 8.0;
const TB_PAD_Y: f32 = 6.0;
const TB_GAP_FROM_REGION: f32 = 12.0;
const TB_RADIUS: f32 = 7.0;
const ICON_PIX: f32 = 20.0;

const RADIAL_CELL: f32 = 26.0;
const RADIAL_GAP: f32 = 4.0;
const RADIAL_COLS: usize = 4;
const RADIAL_PAD: f32 = 8.0;
const RADIAL_RADIUS: f32 = 8.0;

/// Per-app cache of rasterised SVG icons uploaded to the GPU.
#[derive(Default)]
pub(crate) struct IconCache {
    map: HashMap<(ToolbarIcon, [u8; 3]), TextureHandle>,
}

impl IconCache {
    pub(crate) fn get(
        &mut self,
        ctx: &egui::Context,
        icon: ToolbarIcon,
        rgb: [u8; 3],
    ) -> Option<TextureHandle> {
        if let Some(h) = self.map.get(&(icon, rgb)) {
            return Some(h.clone());
        }
        let raster = rasterise_icon(icon, rgb)?;
        let pixels = egui::ColorImage::from_rgba_premultiplied(
            [raster.width as usize, raster.height as usize],
            &raster.rgba,
        );
        let handle = ctx.load_texture(
            format!(
                "sss::icon::{icon:?}::{:02x}{:02x}{:02x}",
                rgb[0], rgb[1], rgb[2]
            ),
            pixels,
            egui::TextureOptions::LINEAR,
        );
        self.map.insert((icon, rgb), handle.clone());
        Some(handle)
    }
}

/// Right-click menu state; window-local pixel origin.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RadialState {
    pub origin: Pos2,
}

/// Toggles forwarded by [`draw_toolbar`] for the driver to apply after the
/// egui pass.
#[derive(Default, Clone, Copy, Debug)]
pub(crate) struct ToolbarOutcome {
    pub confirm: bool,
    pub cancel: bool,
    pub copy: bool,
    pub save: bool,
    pub toggle_pipette: bool,
    pub toggle_snap: bool,
    pub toggle_magnifier: bool,
    pub open_width_popup: Option<Pos2>,
    pub open_snap_popup: Option<Pos2>,
    pub open_color_popup: Option<Pos2>,
    pub raise_selected: bool,
    pub lower_selected: bool,
    pub delete_selected: bool,
    /// `Some(i)` if the user picked tools palette index `i` as outline; the
    /// driver applies `current_color`/`current_width`/clears fill.
    pub select_tool: Option<usize>,
    /// `Some(i)` if the user picked tools palette index `i` as filled.
    pub select_tool_filled: Option<usize>,
    pub undo: bool,
    pub redo: bool,
    pub clear_all: bool,
}

/// Toggle states for the chip buttons (pipette / snap / magnifier).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ToolbarConfig {
    pub pipette_active: bool,
    pub snap_active: bool,
    pub magnifier_active: bool,
    pub snap_step: f32,
}

/// Render the floating toolbar inside `ctx`. Returns user-triggered
/// confirm / cancel / copy / save edges.
///
/// `region` and `monitor_origin` are in global pixel coords; the toolbar
/// is positioned in window-local coords by subtracting `monitor_origin`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_toolbar(
    ctx: &egui::Context,
    canvas: &mut Canvas,
    palette: &ToolPalette,
    _mode: &mut SelectorMode,
    current_color: SssColor,
    current_width: f32,
    monitor_origin: Pos2,
    monitor_size: Vec2,
    region: Option<sss_capture::Rect>,
    chrome: &ChromeColors,
    cfg: ToolbarConfig,
    icons: &mut IconCache,
) -> (ToolbarOutcome, Rect) {
    let mut out = ToolbarOutcome::default();

    // ---- decide which buttons to render ----
    let mut buttons: Vec<Button> = Vec::new();

    let active_disc = std::mem::discriminant(&canvas.active_tool);
    let fill_on = canvas.fill_mode();
    for (i, tool) in palette.tools.iter().enumerate() {
        let is_closed = matches!(
            tool,
            Tool::Rectangle(_) | Tool::Ellipse(_) | Tool::Polygon(_)
        );
        let outlined_active =
            std::mem::discriminant(tool) == active_disc && (!is_closed || !fill_on);
        buttons.push(Button {
            kind: ButtonKind::Tool,
            action: Action::SelectTool(i),
            icon: Some(tool_icon(tool)),
            label: None,
            tint: None,
            bg_tint: None,
            active: outlined_active,
            width: TB_BTN,
        });
        if is_closed {
            let filled_active = std::mem::discriminant(tool) == active_disc && fill_on;
            buttons.push(Button {
                kind: ButtonKind::Tool,
                action: Action::SelectToolFilled(i),
                icon: Some(filled_tool_icon(tool)),
                label: None,
                tint: None,
                bg_tint: None,
                active: filled_active,
                width: TB_BTN,
            });
        }
    }

    // Colour swatch (the radial trigger; on the legacy chip this opened a
    // colour-picker popup — we open the radial menu instead).
    buttons.push(Button {
        kind: ButtonKind::Chip,
        action: Action::OpenColor,
        icon: Some(ToolbarIcon::ColorSwatch),
        label: None,
        tint: Some(contrast_tint([
            current_color.0[0],
            current_color.0[1],
            current_color.0[2],
        ])),
        bg_tint: Some([current_color.0[0], current_color.0[1], current_color.0[2]]),
        active: false,
        width: TB_BTN,
    });
    // Width readout chip (click opens width popup).
    buttons.push(Button {
        kind: ButtonKind::Chip,
        action: Action::OpenWidth,
        icon: None,
        label: Some(format!("{}px", current_width.round() as i32)),
        tint: None,
        bg_tint: None,
        active: false,
        width: TB_BTN + 8.0,
    });

    // Pipette / Snap / Magnifier toggles.
    buttons.push(Button {
        kind: ButtonKind::Chip,
        action: Action::TogglePipette,
        icon: Some(ToolbarIcon::Pipette),
        label: None,
        tint: None,
        bg_tint: None,
        active: cfg.pipette_active,
        width: TB_BTN,
    });
    buttons.push(Button {
        kind: ButtonKind::Chip,
        action: Action::ToggleSnap,
        icon: Some(ToolbarIcon::Snap),
        label: None,
        tint: None,
        bg_tint: None,
        active: cfg.snap_active,
        width: TB_BTN,
    });
    // Snap step popup chip.
    buttons.push(Button {
        kind: ButtonKind::Chip,
        action: Action::OpenSnap,
        icon: None,
        label: Some(format!("{}px", cfg.snap_step.round() as i32)),
        tint: None,
        bg_tint: None,
        active: false,
        width: TB_BTN + 8.0,
    });
    buttons.push(Button {
        kind: ButtonKind::Chip,
        action: Action::ToggleMagnifier,
        icon: Some(ToolbarIcon::Magnifier),
        label: None,
        tint: None,
        bg_tint: None,
        active: cfg.magnifier_active,
        width: TB_BTN,
    });

    // Confirm / copy / save / cancel / undo / redo / clear live on the
    // separate side action toolbar (`draw_action_toolbar`), not here.

    // ---- compute layout ----
    let mut total_w = TB_PAD_X * 2.0;
    let mut prev_kind: Option<ButtonKind> = None;
    for b in &buttons {
        if let Some(prev) = prev_kind {
            total_w += if prev != b.kind { TB_SEP } else { TB_GAP };
        }
        total_w += b.width;
        prev_kind = Some(b.kind);
    }
    let total_h = TB_BTN + TB_PAD_Y * 2.0;

    // anchor — above/below region if any, else top of monitor.
    let (tb_x, tb_y) = anchor(region, monitor_origin, monitor_size, total_w, total_h);

    // ---- render ----
    let bg = Color32::from_rgb(
        chrome.toolbar_bg.0[0],
        chrome.toolbar_bg.0[1],
        chrome.toolbar_bg.0[2],
    );
    let border = Color32::from_rgb(
        chrome.toolbar_border.0[0],
        chrome.toolbar_border.0[1],
        chrome.toolbar_border.0[2],
    );
    let fg = Color32::from_rgb(
        chrome.toolbar_fg.0[0],
        chrome.toolbar_fg.0[1],
        chrome.toolbar_fg.0[2],
    );

    let toolbar_rect = Rect::from_min_size(Pos2::new(tb_x, tb_y), Vec2::new(total_w, total_h));
    let area_resp = egui::Area::new(egui::Id::new("sss::toolbar"))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(tb_x, tb_y))
        .show(ctx, |ui| {
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(total_w, total_h), Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, TB_RADIUS, bg);
            painter.rect_stroke(rect, TB_RADIUS, Stroke::new(1.0, border), egui::StrokeKind::Inside);

            let mut cursor_x = rect.min.x + TB_PAD_X;
            let cursor_y = rect.min.y + TB_PAD_Y;
            let mut prev_kind: Option<ButtonKind> = None;
            for b in buttons.iter() {
                if let Some(prev) = prev_kind {
                    cursor_x += if prev != b.kind { TB_SEP } else { TB_GAP };
                }
                let btn_rect =
                    Rect::from_min_size(Pos2::new(cursor_x, cursor_y), Vec2::new(b.width, TB_BTN));
                draw_button(
                    ui,
                    &painter,
                    btn_rect,
                    b,
                    fg,
                    chrome,
                    icons,
                    &mut out,
                    canvas,
                    palette,
                    _mode,
                );
                cursor_x += b.width;
                prev_kind = Some(b.kind);
            }
        });

    let _ = area_resp;
    (out, toolbar_rect)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ButtonKind {
    Tool,
    Chip,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
// Some variants only exist for outcome propagation (the click handler
// matches on them) and aren't constructed today since their chips moved
// to the side / selection toolbars. Kept around so the enum stays
// expressive if we re-add them.
#[allow(dead_code)]
enum Action {
    SelectTool(usize),
    SelectToolFilled(usize),
    Undo,
    Redo,
    ClearAll,
    Cancel,
    Confirm,
    Copy,
    Save,
    OpenColor,
    OpenWidth,
    OpenSnap,
    TogglePipette,
    ToggleSnap,
    ToggleMagnifier,
    RaiseSelected,
    LowerSelected,
    DeleteSelected,
}

struct Button {
    kind: ButtonKind,
    action: Action,
    icon: Option<ToolbarIcon>,
    label: Option<String>,
    /// Tint for the icon glyph. Used by the colour swatch chip to draw the
    /// SVG silhouette in the user's current colour.
    tint: Option<[u8; 3]>,
    /// Optional background colour for the button. Used by the colour
    /// swatch chip so the chip itself reads as the current colour.
    bg_tint: Option<[u8; 3]>,
    active: bool,
    width: f32,
}

impl Button {
    #[allow(dead_code)]
    fn action(action: Action, icon: ToolbarIcon) -> Self {
        Self {
            kind: ButtonKind::Chip,
            action,
            icon: Some(icon),
            label: None,
            tint: None,
            bg_tint: None,
            active: false,
            width: TB_BTN,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_button(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    b: &Button,
    fg: Color32,
    chrome: &ChromeColors,
    icons: &mut IconCache,
    out: &mut ToolbarOutcome,
    _canvas: &mut Canvas,
    _palette: &ToolPalette,
    _mode: &mut SelectorMode,
) {
    let resp = ui.interact(rect, egui::Id::new(("sss::tb_btn", &b.action)), Sense::click());

    let hovered = resp.hovered();
    let active = b.active;
    let bg = if let Some(rgb) = b.bg_tint {
        // Chip carries an explicit colour swatch (e.g. the colour-picker
        // button); brighten it slightly on hover.
        if hovered {
            Color32::from_rgb(
                rgb[0].saturating_add(20),
                rgb[1].saturating_add(20),
                rgb[2].saturating_add(20),
            )
        } else {
            Color32::from_rgb(rgb[0], rgb[1], rgb[2])
        }
    } else if active {
        Color32::from_rgb(
            chrome.button_active_bg.0[0],
            chrome.button_active_bg.0[1],
            chrome.button_active_bg.0[2],
        )
    } else if hovered {
        Color32::from_rgb(
            chrome.button_bg.0[0].saturating_add(20),
            chrome.button_bg.0[1].saturating_add(20),
            chrome.button_bg.0[2].saturating_add(20),
        )
    } else {
        Color32::from_rgb(
            chrome.button_bg.0[0],
            chrome.button_bg.0[1],
            chrome.button_bg.0[2],
        )
    };
    painter.rect_filled(rect, 4.0, bg);
    if active {
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(
                1.0,
                Color32::from_rgb(
                    chrome.button_active_border.0[0],
                    chrome.button_active_border.0[1],
                    chrome.button_active_border.0[2],
                ),
            ),
            egui::StrokeKind::Inside,
        );
    }

    if let Some(icon) = b.icon {
        let rgb = b.tint.unwrap_or([fg.r(), fg.g(), fg.b()]);
        if let Some(tex) = icons.get(ui.ctx(), icon, rgb) {
            let icon_size = Vec2::splat(ICON_PIX);
            let icon_rect = Rect::from_center_size(rect.center(), icon_size);
            painter.image(
                tex.id(),
                icon_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    } else if let Some(label) = b.label.as_ref() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            FontId::proportional(11.0),
            fg,
        );
    }

    if resp.clicked() {
        match &b.action {
            Action::SelectTool(i) => out.select_tool = Some(*i),
            Action::SelectToolFilled(i) => out.select_tool_filled = Some(*i),
            Action::Undo => out.undo = true,
            Action::Redo => out.redo = true,
            Action::ClearAll => out.clear_all = true,
            Action::Cancel => out.cancel = true,
            Action::Confirm => out.confirm = true,
            Action::Copy => {
                out.copy = true;
                out.confirm = true;
            }
            Action::Save => {
                out.save = true;
                out.confirm = true;
            }
            Action::OpenColor => out.open_color_popup = Some(rect.center_bottom()),
            Action::OpenWidth => out.open_width_popup = Some(rect.center_bottom()),
            Action::OpenSnap => out.open_snap_popup = Some(rect.center_bottom()),
            Action::TogglePipette => out.toggle_pipette = true,
            Action::ToggleSnap => out.toggle_snap = true,
            Action::ToggleMagnifier => out.toggle_magnifier = true,
            Action::RaiseSelected => out.raise_selected = true,
            Action::LowerSelected => out.lower_selected = true,
            Action::DeleteSelected => out.delete_selected = true,
        }
    }
}

fn anchor(
    region: Option<sss_capture::Rect>,
    monitor_origin: Pos2,
    monitor_size: Vec2,
    total_w: f32,
    total_h: f32,
) -> (f32, f32) {
    let mon_w = monitor_size.x;
    let mon_h = monitor_size.y;
    if let Some(r) = region.filter(|r| r.width() >= 2 && r.height() >= 2) {
        let lx = (r.x() as f32) - monitor_origin.x;
        let ly = (r.y() as f32) - monitor_origin.y;
        let lw = r.width() as f32;
        let lh = r.height() as f32;
        let tb_x = (lx + (lw - total_w) / 2.0).clamp(8.0, (mon_w - total_w - 8.0).max(8.0));
        let above = ly - total_h - TB_GAP_FROM_REGION;
        let below = ly + lh + TB_GAP_FROM_REGION;
        let tb_y = if above >= 8.0 {
            above
        } else if below + total_h <= mon_h - 8.0 {
            below
        } else {
            8.0
        };
        (tb_x, tb_y.max(8.0).min((mon_h - total_h - 8.0).max(8.0)))
    } else {
        // Centered at the top of the monitor.
        (
            ((mon_w - total_w) / 2.0).max(8.0),
            12.0,
        )
    }
}

// ============================================================================
// Radial menu
// ============================================================================

/// Result of a radial click.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RadialPick {
    Color(SssColor),
    Width(f32),
}

#[derive(Default, Clone, Copy, Debug)]
pub(crate) struct RadialOutcome {
    pub pick: Option<RadialPick>,
    /// Click landed outside the radial; caller should close it.
    pub close: bool,
}

pub(crate) fn draw_radial(
    ctx: &egui::Context,
    state: &RadialState,
    armed: &mut bool,
    palette: &[SssColor],
    widths: &[f32],
    current_color: SssColor,
    current_width: f32,
    chrome: &ChromeColors,
) -> (RadialOutcome, Rect) {
    let cols = RADIAL_COLS;
    let n_colors = palette.len().max(1);
    let rows = (n_colors + cols - 1) / cols;
    let grid_w = cols as f32 * RADIAL_CELL + (cols as f32 - 1.0) * RADIAL_GAP;
    let grid_h = rows as f32 * RADIAL_CELL + (rows as f32 - 1.0) * RADIAL_GAP;
    let widths_row_h = RADIAL_CELL;
    let total_w = grid_w + RADIAL_PAD * 2.0;
    let total_h = grid_h + RADIAL_GAP + widths_row_h + RADIAL_PAD * 2.0;

    let bg = Color32::from_rgb(
        chrome.toolbar_bg.0[0],
        chrome.toolbar_bg.0[1],
        chrome.toolbar_bg.0[2],
    );
    let border = Color32::from_rgb(
        chrome.toolbar_border.0[0],
        chrome.toolbar_border.0[1],
        chrome.toolbar_border.0[2],
    );
    let active_border = Color32::from_rgb(
        chrome.button_active_border.0[0],
        chrome.button_active_border.0[1],
        chrome.button_active_border.0[2],
    );

    let mut outcome = RadialOutcome::default();
    let radial_rect = Rect::from_min_size(state.origin, Vec2::new(total_w, total_h));

    egui::Area::new(egui::Id::new("sss::radial"))
        .order(egui::Order::Foreground)
        .fixed_pos(state.origin)
        .show(ctx, |ui| {
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(total_w, total_h), Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, RADIAL_RADIUS, bg);
            painter.rect_stroke(
                rect,
                RADIAL_RADIUS,
                Stroke::new(1.0, border),
                egui::StrokeKind::Inside,
            );

            let hover_pos = ui.input(|i| i.pointer.hover_pos());
            let clicked = resp.clicked();

            // colour grid
            for i in 0..n_colors {
                let col = (i % cols) as f32;
                let row = (i / cols) as f32;
                let cx = rect.min.x + RADIAL_PAD + col * (RADIAL_CELL + RADIAL_GAP);
                let cy = rect.min.y + RADIAL_PAD + row * (RADIAL_CELL + RADIAL_GAP);
                let cell = Rect::from_min_size(Pos2::new(cx, cy), Vec2::splat(RADIAL_CELL));
                let c = palette[i];
                let fill = Color32::from_rgba_unmultiplied(c.0[0], c.0[1], c.0[2], c.0[3]);
                painter.rect_filled(cell, 4.0, fill);
                let is_current = c.0 == current_color.0;
                let is_hovered = hover_pos.map_or(false, |p| cell.contains(p));
                if is_current || is_hovered {
                    let stroke = if is_current {
                        Stroke::new(2.0, active_border)
                    } else {
                        Stroke::new(1.5, Color32::WHITE)
                    };
                    painter.rect_stroke(cell, 4.0, stroke, egui::StrokeKind::Inside);
                }
                if clicked && is_hovered {
                    outcome.pick = Some(RadialPick::Color(c));
                }
            }

            // width row
            let widths_y =
                rect.min.y + RADIAL_PAD + grid_h + RADIAL_GAP;
            for (i, w) in widths.iter().enumerate() {
                let cx = rect.min.x + RADIAL_PAD + i as f32 * (RADIAL_CELL + RADIAL_GAP);
                let cell = Rect::from_min_size(Pos2::new(cx, widths_y), Vec2::splat(RADIAL_CELL));
                let is_current = (*w - current_width).abs() < 0.5;
                let is_hovered = hover_pos.map_or(false, |p| cell.contains(p));
                let fill = if is_current {
                    Color32::from_rgb(
                        chrome.button_active_bg.0[0],
                        chrome.button_active_bg.0[1],
                        chrome.button_active_bg.0[2],
                    )
                } else if is_hovered {
                    Color32::from_rgb(
                        chrome.button_bg.0[0].saturating_add(30),
                        chrome.button_bg.0[1].saturating_add(30),
                        chrome.button_bg.0[2].saturating_add(30),
                    )
                } else {
                    Color32::from_rgb(
                        chrome.button_bg.0[0],
                        chrome.button_bg.0[1],
                        chrome.button_bg.0[2],
                    )
                };
                painter.rect_filled(cell, 4.0, fill);
                let dot_r = (w * 1.4).min(RADIAL_CELL * 0.4).max(2.0);
                painter.circle_filled(
                    cell.center(),
                    dot_r,
                    Color32::from_rgb(
                        chrome.toolbar_fg.0[0],
                        chrome.toolbar_fg.0[1],
                        chrome.toolbar_fg.0[2],
                    ),
                );
                if clicked && is_hovered {
                    outcome.pick = Some(RadialPick::Width(*w));
                }
            }

            // Skip the click that opened the popup — only honour
            // outside-click-close after the popup has been visible for at
            // least one frame.
            if !*armed {
                *armed = true;
            } else if !resp.clicked() && ui.input(|i| i.pointer.any_click()) {
                if let Some(p) = hover_pos {
                    if !rect.contains(p) {
                        outcome.close = true;
                    }
                }
            }
        });

    (outcome, radial_rect)
}

/// Apply a radial pick to the active tool / canvas state. Returns the
/// updated `(current_color, current_width)` pair for the caller to mirror
/// back into `App`. Kept for ergonomic re-use by callers that don't want
/// to repeat the match.
#[allow(dead_code)]
pub(crate) fn apply_radial_pick(
    canvas: &mut Canvas,
    current_color: SssColor,
    current_width: f32,
    pick: RadialPick,
) -> (SssColor, f32) {
    let mut color = current_color;
    let mut width = current_width;
    match pick {
        RadialPick::Color(c) => {
            color = c;
            apply_tool_color(&mut canvas.active_tool, c);
            if canvas.fill_mode() {
                canvas.set_fill_color(Some(c));
            }
        }
        RadialPick::Width(w) => {
            width = w;
            set_active_tool_width(&mut canvas.active_tool, w);
        }
    }
    (color, width)
}

// ============================================================================
// Width / Snap popups
// ============================================================================

const POPUP_W: f32 = 220.0;
const POPUP_H: f32 = 54.0;
const POPUP_PAD: f32 = 10.0;

/// Outcome from a slider popup.
#[derive(Default, Clone, Copy, Debug)]
pub(crate) struct SliderPopupOutcome {
    pub value: Option<f32>,
    pub close: bool,
}

/// Generic drag-slider popup. `value` mutates in place when dragged.
pub(crate) fn draw_slider_popup(
    ctx: &egui::Context,
    id: &str,
    origin: Pos2,
    armed: &mut bool,
    label: &str,
    min: f32,
    max: f32,
    current: f32,
    chrome: &ChromeColors,
) -> (SliderPopupOutcome, Rect) {
    let mut out = SliderPopupOutcome::default();
    let popup_rect = Rect::from_min_size(
        Pos2::new(origin.x - POPUP_W / 2.0, origin.y + 4.0),
        Vec2::new(POPUP_W, POPUP_H),
    );
    let bg = Color32::from_rgb(
        chrome.toolbar_bg.0[0],
        chrome.toolbar_bg.0[1],
        chrome.toolbar_bg.0[2],
    );
    let border = Color32::from_rgb(
        chrome.toolbar_border.0[0],
        chrome.toolbar_border.0[1],
        chrome.toolbar_border.0[2],
    );
    let fg = Color32::from_rgb(
        chrome.toolbar_fg.0[0],
        chrome.toolbar_fg.0[1],
        chrome.toolbar_fg.0[2],
    );
    let accent = Color32::from_rgb(
        chrome.button_active_bg.0[0],
        chrome.button_active_bg.0[1],
        chrome.button_active_bg.0[2],
    );

    egui::Area::new(egui::Id::new(format!("sss::popup::{id}")))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(origin.x - POPUP_W / 2.0, origin.y + 4.0))
        .show(ctx, |ui| {
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(POPUP_W, POPUP_H), Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 6.0, bg);
            painter.rect_stroke(rect, 6.0, Stroke::new(1.0, border), egui::StrokeKind::Inside);

            painter.text(
                Pos2::new(rect.min.x + POPUP_PAD, rect.min.y + POPUP_PAD),
                egui::Align2::LEFT_TOP,
                format!("{label}: {:.0}px", current),
                FontId::proportional(11.0),
                fg,
            );

            // Slider track + knob.
            let track_y = rect.min.y + POPUP_H - 16.0;
            let track_x0 = rect.min.x + POPUP_PAD;
            let track_x1 = rect.max.x - POPUP_PAD;
            let track_rect = Rect::from_min_max(
                Pos2::new(track_x0, track_y - 3.0),
                Pos2::new(track_x1, track_y + 3.0),
            );
            painter.rect_filled(track_rect, 3.0, Color32::from_rgb(70, 70, 74));
            let t = ((current - min) / (max - min)).clamp(0.0, 1.0);
            let knob_x = track_x0 + t * (track_x1 - track_x0);
            painter.circle_filled(Pos2::new(knob_x, track_y), 7.0, accent);
            painter.circle_stroke(
                Pos2::new(knob_x, track_y),
                7.0,
                Stroke::new(1.0, Color32::WHITE),
            );

            // Drag / click on track sets value.
            if resp.dragged() || resp.clicked() {
                if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                    if p.y >= rect.min.y + POPUP_H - 28.0 {
                        let t = ((p.x - track_x0) / (track_x1 - track_x0)).clamp(0.0, 1.0);
                        out.value = Some(min + t * (max - min));
                    }
                }
            }

            // Click outside popup closes it (after the opening click).
            if !*armed {
                *armed = true;
            } else if !resp.clicked()
                && !resp.dragged()
                && ui.input(|i| i.pointer.any_click())
            {
                if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                    if !rect.contains(p) {
                        out.close = true;
                    }
                }
            }
        });

    (out, popup_rect)
}

// ============================================================================
// HSV color picker popup
// ============================================================================

const HSV_W: f32 = 220.0;
const HSV_SV_H: f32 = 140.0;
const HSV_HUE_H: f32 = 14.0;
const HSV_PAD: f32 = 10.0;

#[derive(Default, Clone, Copy, Debug)]
pub(crate) struct ColorPopupOutcome {
    pub color: Option<SssColor>,
    pub close: bool,
}

/// Persistent HSV state so the popup keeps its hue when only S/V is dragged.
/// The driver owns one and feeds it in/out.
#[derive(Clone, Debug)]
pub(crate) struct HsvState {
    pub hue: f32,
    pub sat: f32,
    pub val: f32,
    /// Hex text buffer for the in-popup `#RRGGBB` input. Kept in state so
    /// typing isn't lost between frames; rewritten when the user drags the
    /// SV / hue widgets.
    pub hex_buffer: String,
}

impl HsvState {
    pub fn from_rgb(c: SssColor) -> Self {
        let (h, s, v) = rgb_to_hsv(c.0[0], c.0[1], c.0[2]);
        let mut s = Self {
            hue: h,
            sat: s,
            val: v,
            hex_buffer: String::new(),
        };
        s.refresh_hex();
        s
    }
    pub fn to_rgb(&self) -> [u8; 3] {
        hsv_to_rgb(self.hue, self.sat, self.val)
    }
    /// Sync the hex buffer from the current HSV.
    pub fn refresh_hex(&mut self) {
        let rgb = self.to_rgb();
        self.hex_buffer = format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
    }
}

/// Parse a `#RRGGBB` / `RRGGBB` hex string into RGB bytes. Tolerates
/// leading hash + surrounding whitespace.
/// Pick black or white for an icon glyph so it stays readable on top of
/// the given background colour. Uses the standard luminance threshold.
fn contrast_tint(bg: [u8; 3]) -> [u8; 3] {
    let lum =
        0.2126 * bg[0] as f32 + 0.7152 * bg[1] as f32 + 0.0722 * bg[2] as f32;
    if lum > 140.0 {
        [20, 20, 20]
    } else {
        [240, 240, 240]
    }
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some([r, g, b])
}

const HSV_FOOTER_H: f32 = 26.0;

pub(crate) fn draw_color_popup(
    ctx: &egui::Context,
    origin: Pos2,
    armed: &mut bool,
    state: &mut HsvState,
    chrome: &ChromeColors,
) -> (ColorPopupOutcome, Rect) {
    let mut out = ColorPopupOutcome::default();
    let total_h = HSV_SV_H + HSV_HUE_H + HSV_FOOTER_H + HSV_PAD * 4.0;
    let popup_rect = Rect::from_min_size(
        Pos2::new(origin.x - HSV_W / 2.0, origin.y + 4.0),
        Vec2::new(HSV_W, total_h),
    );

    let bg = Color32::from_rgb(
        chrome.toolbar_bg.0[0],
        chrome.toolbar_bg.0[1],
        chrome.toolbar_bg.0[2],
    );
    let border = Color32::from_rgb(
        chrome.toolbar_border.0[0],
        chrome.toolbar_border.0[1],
        chrome.toolbar_border.0[2],
    );

    egui::Area::new(egui::Id::new("sss::popup::color"))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(origin.x - HSV_W / 2.0, origin.y + 4.0))
        .show(ctx, |ui| {
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(HSV_W, total_h), Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 6.0, bg);
            painter.rect_stroke(rect, 6.0, Stroke::new(1.0, border), egui::StrokeKind::Inside);

            // S/V quad at top.
            let sv_rect = Rect::from_min_size(
                Pos2::new(rect.min.x + HSV_PAD, rect.min.y + HSV_PAD),
                Vec2::new(HSV_W - HSV_PAD * 2.0, HSV_SV_H),
            );
            // Rasterise an SV gradient as a 16x16 mesh tinted by hue.
            const N: usize = 16;
            for j in 0..N {
                for i in 0..N {
                    let s0 = i as f32 / N as f32;
                    let s1 = (i + 1) as f32 / N as f32;
                    let v0 = 1.0 - (j as f32 / N as f32);
                    let v1 = 1.0 - ((j + 1) as f32 / N as f32);
                    let rgb = hsv_to_rgb(state.hue, (s0 + s1) * 0.5, (v0 + v1) * 0.5);
                    let cell = Rect::from_min_max(
                        Pos2::new(
                            sv_rect.min.x + s0 * sv_rect.width(),
                            sv_rect.min.y + (1.0 - v0) * sv_rect.height(),
                        ),
                        Pos2::new(
                            sv_rect.min.x + s1 * sv_rect.width(),
                            sv_rect.min.y + (1.0 - v1) * sv_rect.height(),
                        ),
                    );
                    painter.rect_filled(cell, 0.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                }
            }
            painter.rect_stroke(
                sv_rect,
                0.0,
                Stroke::new(1.0, border),
                egui::StrokeKind::Inside,
            );
            // S/V marker.
            let mk = Pos2::new(
                sv_rect.min.x + state.sat * sv_rect.width(),
                sv_rect.min.y + (1.0 - state.val) * sv_rect.height(),
            );
            painter.circle_stroke(mk, 5.0, Stroke::new(2.0, Color32::WHITE));
            painter.circle_stroke(mk, 5.0, Stroke::new(1.0, Color32::BLACK));

            // Hue strip below.
            let hue_rect = Rect::from_min_size(
                Pos2::new(rect.min.x + HSV_PAD, sv_rect.max.y + HSV_PAD),
                Vec2::new(HSV_W - HSV_PAD * 2.0, HSV_HUE_H),
            );
            const HN: usize = 60;
            for i in 0..HN {
                let h0 = i as f32 / HN as f32;
                let h1 = (i + 1) as f32 / HN as f32;
                let rgb = hsv_to_rgb((h0 + h1) * 0.5, 1.0, 1.0);
                let cell = Rect::from_min_max(
                    Pos2::new(hue_rect.min.x + h0 * hue_rect.width(), hue_rect.min.y),
                    Pos2::new(hue_rect.min.x + h1 * hue_rect.width(), hue_rect.max.y),
                );
                painter.rect_filled(cell, 0.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
            }
            painter.rect_stroke(
                hue_rect,
                0.0,
                Stroke::new(1.0, border),
                egui::StrokeKind::Inside,
            );
            // Hue marker.
            let hx = hue_rect.min.x + state.hue * hue_rect.width();
            painter.line_segment(
                [Pos2::new(hx, hue_rect.min.y - 2.0), Pos2::new(hx, hue_rect.max.y + 2.0)],
                Stroke::new(2.0, Color32::WHITE),
            );

            // Interactions: drag inside either rect updates state.
            let mut changed_via_widget = false;
            if resp.dragged() || resp.clicked() {
                if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                    if sv_rect.contains(p) {
                        state.sat = ((p.x - sv_rect.min.x) / sv_rect.width()).clamp(0.0, 1.0);
                        state.val =
                            (1.0 - (p.y - sv_rect.min.y) / sv_rect.height()).clamp(0.0, 1.0);
                        let rgb = state.to_rgb();
                        out.color = Some(SssColor::rgb(rgb[0], rgb[1], rgb[2]));
                        changed_via_widget = true;
                    } else if hue_rect.contains(p) {
                        state.hue =
                            ((p.x - hue_rect.min.x) / hue_rect.width()).clamp(0.0, 1.0);
                        let rgb = state.to_rgb();
                        out.color = Some(SssColor::rgb(rgb[0], rgb[1], rgb[2]));
                        changed_via_widget = true;
                    }
                }
            }
            if changed_via_widget {
                state.refresh_hex();
            }

            // Footer row: swatch preview + editable hex input.
            let footer_y = hue_rect.max.y + HSV_PAD;
            let swatch_rect = Rect::from_min_size(
                Pos2::new(rect.min.x + HSV_PAD, footer_y),
                Vec2::new(HSV_FOOTER_H, HSV_FOOTER_H),
            );
            let [r, g, b] = state.to_rgb();
            painter.rect_filled(swatch_rect, 3.0, Color32::from_rgb(r, g, b));
            painter.rect_stroke(
                swatch_rect,
                3.0,
                Stroke::new(1.0, border),
                egui::StrokeKind::Inside,
            );
            let hex_rect = Rect::from_min_max(
                Pos2::new(swatch_rect.max.x + HSV_PAD, footer_y),
                Pos2::new(rect.max.x - HSV_PAD, footer_y + HSV_FOOTER_H),
            );
            // egui needs a real Ui to lay out a TextEdit. `child_ui_with_id_source`
            // gives us one scoped to `hex_rect`.
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(hex_rect)
                    .id_salt("sss::popup::color::hex"),
            );
            let edit_resp = child.add_sized(
                hex_rect.size(),
                egui::TextEdit::singleline(&mut state.hex_buffer)
                    .font(FontId::monospace(13.0))
                    .horizontal_align(egui::Align::Center),
            );
            if edit_resp.changed() {
                if let Some([r2, g2, b2]) = parse_hex(&state.hex_buffer) {
                    let (h, s, v) = rgb_to_hsv(r2, g2, b2);
                    state.hue = h;
                    state.sat = s;
                    state.val = v;
                    out.color = Some(SssColor::rgb(r2, g2, b2));
                }
            }

            // Click outside closes (only after the opening click).
            if !*armed {
                *armed = true;
            } else if !resp.clicked()
                && !resp.dragged()
                && ui.input(|i| i.pointer.any_click())
            {
                if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                    if !rect.contains(p) {
                        out.close = true;
                    }
                }
            }
        });

    (out, popup_rect)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = v * s;
    let h6 = (h * 6.0).rem_euclid(6.0);
    let x = c * (1.0 - ((h6 % 2.0) - 1.0).abs());
    let (r, g, b) = match h6 as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [
        ((r + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((g + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((b + m) * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let d = max - min;
    let v = max;
    let s = if max <= 0.0 { 0.0 } else { d / max };
    let h = if d <= 0.0 {
        0.0
    } else if max == rf {
        ((gf - bf) / d).rem_euclid(6.0) / 6.0
    } else if max == gf {
        ((bf - rf) / d + 2.0) / 6.0
    } else {
        ((rf - gf) / d + 4.0) / 6.0
    };
    (h, s, v)
}

// ============================================================================
// Magnifier
// ============================================================================

/// Draw a circular magnifier of the eager capture around `pointer_local`.
/// `bg_tex` is the per-monitor egui texture already uploaded for the
/// background; `monitor_size` is the surface size in window-local px.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_magnifier(
    ctx: &egui::Context,
    bg_tex: &TextureHandle,
    pointer_local: Pos2,
    monitor_size: Vec2,
    zoom: f32,
    chrome: &ChromeColors,
    hex_label: Option<String>,
) {
    const RADIUS: f32 = 70.0;
    const MARGIN: f32 = 18.0;

    // Place the magnifier so it doesn't sit under the cursor.
    let mut cx = pointer_local.x + RADIUS + MARGIN;
    let mut cy = pointer_local.y + RADIUS + MARGIN;
    if cx + RADIUS > monitor_size.x - 4.0 {
        cx = pointer_local.x - RADIUS - MARGIN;
    }
    if cy + RADIUS > monitor_size.y - 4.0 {
        cy = pointer_local.y - RADIUS - MARGIN;
    }
    cx = cx.clamp(RADIUS + 4.0, monitor_size.x - RADIUS - 4.0);
    cy = cy.clamp(RADIUS + 4.0, monitor_size.y - RADIUS - 4.0);
    let center = Pos2::new(cx, cy);

    let border = Color32::from_rgb(
        chrome.toolbar_border.0[0],
        chrome.toolbar_border.0[1],
        chrome.toolbar_border.0[2],
    );

    egui::Area::new(egui::Id::new("sss::magnifier"))
        .order(egui::Order::Tooltip)
        .fixed_pos(Pos2::new(center.x - RADIUS, center.y - RADIUS))
        .show(ctx, |ui| {
            let rect = Rect::from_center_size(center, Vec2::splat(RADIUS * 2.0));
            let painter = ui.painter_at(rect.expand(40.0));

            // Build a circular fan mesh whose UVs sample a small disc
            // around the pointer from the background texture, scaled by
            // `zoom`. This gives a real circular magnifier (not a square
            // image clipped by an outline).
            let segments = 96usize;
            let sample_half = RADIUS / zoom;
            let mut mesh = egui::Mesh::with_texture(bg_tex.id());
            // Center vertex.
            let uv_cx = (pointer_local.x / monitor_size.x).clamp(0.0, 1.0);
            let uv_cy = (pointer_local.y / monitor_size.y).clamp(0.0, 1.0);
            mesh.vertices.push(egui::epaint::Vertex {
                pos: center,
                uv: Pos2::new(uv_cx, uv_cy),
                color: Color32::WHITE,
            });
            for i in 0..=segments {
                let theta = (i as f32 / segments as f32) * std::f32::consts::TAU;
                let dx = theta.cos();
                let dy = theta.sin();
                let vx = center.x + dx * RADIUS;
                let vy = center.y + dy * RADIUS;
                let u = ((pointer_local.x + dx * sample_half) / monitor_size.x).clamp(0.0, 1.0);
                let v = ((pointer_local.y + dy * sample_half) / monitor_size.y).clamp(0.0, 1.0);
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: Pos2::new(vx, vy),
                    uv: Pos2::new(u, v),
                    color: Color32::WHITE,
                });
            }
            for i in 0..segments {
                mesh.indices.push(0);
                mesh.indices.push((i + 1) as u32);
                mesh.indices.push((i + 2) as u32);
            }
            painter.add(egui::Shape::mesh(mesh));

            painter.circle_stroke(center, RADIUS, Stroke::new(2.0, border));
            painter.line_segment(
                [
                    Pos2::new(center.x - 8.0, center.y),
                    Pos2::new(center.x + 8.0, center.y),
                ],
                Stroke::new(1.0, Color32::WHITE),
            );
            painter.line_segment(
                [
                    Pos2::new(center.x, center.y - 8.0),
                    Pos2::new(center.x, center.y + 8.0),
                ],
                Stroke::new(1.0, Color32::WHITE),
            );
            if let Some(hex) = hex_label {
                let lbl_rect = Rect::from_center_size(
                    Pos2::new(center.x, center.y + RADIUS + 14.0),
                    Vec2::new(86.0, 18.0),
                );
                painter.rect_filled(
                    lbl_rect,
                    3.0,
                    Color32::from_rgba_unmultiplied(20, 20, 22, 230),
                );
                painter.rect_stroke(
                    lbl_rect,
                    3.0,
                    Stroke::new(1.0, border),
                    egui::StrokeKind::Inside,
                );
                painter.text(
                    lbl_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    hex,
                    FontId::monospace(12.0),
                    Color32::from_rgb(240, 240, 240),
                );
            }
        });
}

// ============================================================================
// Selection toolbar (raise / lower / trash) attached to the selected shape
// ============================================================================

const SEL_BTN: f32 = 24.0;
const SEL_BTN_GAP: f32 = 4.0;
const SEL_BTN_PAD: f32 = 6.0;
const SEL_BAR_OFFSET: f32 = 10.0;

#[derive(Default, Clone, Copy, Debug)]
pub(crate) struct SelectionDecorOutcome {
    pub raise: bool,
    pub lower: bool,
    pub delete: bool,
}

/// Render the selection toolbar (raise / lower / trash) anchored near the
/// shape's bounds. Returns the action flags + the rect occupied by the
/// bar so the driver can hit-test events against it.
pub(crate) fn draw_selection_toolbar(
    ctx: &egui::Context,
    bounds: sss_capture::Rect,
    monitor_origin: Pos2,
    monitor_size: Vec2,
    chrome: &ChromeColors,
    icons: &mut IconCache,
) -> (SelectionDecorOutcome, Rect) {
    let mut out = SelectionDecorOutcome::default();

    let bw = bounds.width() as f32;
    let bh = bounds.height() as f32;
    let lx = bounds.x() as f32 - monitor_origin.x;
    let ly = bounds.y() as f32 - monitor_origin.y;

    let entries: [(crate::icons::ToolbarIcon, &str); 3] = [
        (crate::icons::ToolbarIcon::Raise, "raise"),
        (crate::icons::ToolbarIcon::Lower, "lower"),
        (crate::icons::ToolbarIcon::Trash, "trash"),
    ];
    let n = entries.len() as f32;
    let total_w = SEL_BTN_PAD * 2.0 + n * SEL_BTN + (n - 1.0) * SEL_BTN_GAP;
    let total_h = SEL_BTN_PAD * 2.0 + SEL_BTN;

    // Prefer above; fall back to below; clamp inside monitor.
    let mut bar_x = lx + bw - total_w + 6.0;
    bar_x = bar_x.clamp(8.0, (monitor_size.x - total_w - 8.0).max(8.0));
    let above = ly - total_h - SEL_BAR_OFFSET;
    let below = ly + bh + SEL_BAR_OFFSET;
    let bar_y = if above >= 8.0 {
        above
    } else if below + total_h <= monitor_size.y - 8.0 {
        below
    } else {
        8.0
    }
    .clamp(8.0, (monitor_size.y - total_h - 8.0).max(8.0));

    let bar_rect = Rect::from_min_size(Pos2::new(bar_x, bar_y), Vec2::new(total_w, total_h));
    let bg = Color32::from_rgb(
        chrome.toolbar_bg.0[0],
        chrome.toolbar_bg.0[1],
        chrome.toolbar_bg.0[2],
    );
    let border = Color32::from_rgb(
        chrome.toolbar_border.0[0],
        chrome.toolbar_border.0[1],
        chrome.toolbar_border.0[2],
    );
    let fg = Color32::from_rgb(
        chrome.toolbar_fg.0[0],
        chrome.toolbar_fg.0[1],
        chrome.toolbar_fg.0[2],
    );

    egui::Area::new(egui::Id::new("sss::selection_toolbar"))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(bar_x, bar_y))
        .interactable(true)
        .show(ctx, |ui| {
            let painter = ui.painter();
            painter.rect_filled(bar_rect, 6.0, bg);
            painter.rect_stroke(
                bar_rect,
                6.0,
                Stroke::new(1.0, border),
                egui::StrokeKind::Inside,
            );

            for (i, (icon, name)) in entries.iter().enumerate() {
                let bx = bar_rect.min.x + SEL_BTN_PAD + i as f32 * (SEL_BTN + SEL_BTN_GAP);
                let by = bar_rect.min.y + SEL_BTN_PAD;
                let btn_rect = Rect::from_min_size(Pos2::new(bx, by), Vec2::splat(SEL_BTN));
                let resp = ui.interact(
                    btn_rect,
                    egui::Id::new(("sss::sel_btn", *name)),
                    Sense::click(),
                );
                let bgc = if resp.hovered() {
                    Color32::from_rgb(
                        chrome.button_bg.0[0].saturating_add(20),
                        chrome.button_bg.0[1].saturating_add(20),
                        chrome.button_bg.0[2].saturating_add(20),
                    )
                } else {
                    Color32::from_rgb(
                        chrome.button_bg.0[0],
                        chrome.button_bg.0[1],
                        chrome.button_bg.0[2],
                    )
                };
                painter.rect_filled(btn_rect, 4.0, bgc);
                let tint = if matches!(icon, crate::icons::ToolbarIcon::Trash) {
                    [220, 70, 70]
                } else {
                    [fg.r(), fg.g(), fg.b()]
                };
                if let Some(tex) = icons.get(ui.ctx(), *icon, tint) {
                    let icon_rect = Rect::from_center_size(btn_rect.center(), Vec2::splat(18.0));
                    painter.image(
                        tex.id(),
                        icon_rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                if resp.clicked() {
                    match icon {
                        crate::icons::ToolbarIcon::Raise => out.raise = true,
                        crate::icons::ToolbarIcon::Lower => out.lower = true,
                        crate::icons::ToolbarIcon::Trash => out.delete = true,
                        _ => {}
                    }
                }
            }
            ui.allocate_rect(bar_rect, Sense::hover());
        });

    (out, bar_rect)
}

// ============================================================================
// Transform gizmos (selected shape)
// ============================================================================

/// Which handle on the selection box is under the pointer (if any).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GizmoHandle {
    Scale,
    Rotate,
}

const GIZMO_HANDLE_R: f32 = 7.0;
const GIZMO_ROTATE_OFFSET: f32 = 28.0;

/// Hit-test the gizmo handles in `global` coords. Returns the handle if
/// `point` is inside one. `bounds` are the shape bounds in global px.
pub(crate) fn hit_gizmo(
    bounds: sss_capture::Rect,
    point: crate::geometry::FPoint,
) -> Option<GizmoHandle> {
    let br = Pos2::new(
        (bounds.x() + bounds.width() as i32) as f32,
        (bounds.y() + bounds.height() as i32) as f32,
    );
    let rot = Pos2::new(
        bounds.x() as f32 + bounds.width() as f32 / 2.0,
        bounds.y() as f32 - GIZMO_ROTATE_OFFSET,
    );
    let p = Pos2::new(point.x, point.y);
    let d_br = (p - br).length();
    let d_rot = (p - rot).length();
    if d_rot <= GIZMO_HANDLE_R + 2.0 {
        Some(GizmoHandle::Rotate)
    } else if d_br <= GIZMO_HANDLE_R + 2.0 {
        Some(GizmoHandle::Scale)
    } else {
        None
    }
}

/// Draw selection outline + handles (scale at bottom-right, rotate above
/// the top edge). Painted directly on the central canvas painter.
// ============================================================================
// Side action toolbar (undo / redo / clear / confirm / copy / save / cancel)
// ============================================================================

#[derive(Default, Clone, Copy, Debug)]
pub(crate) struct ActionToolbarOutcome {
    pub undo: bool,
    pub redo: bool,
    pub border_toggle: bool,
    pub clear_all: bool,
    pub confirm: bool,
    pub copy: bool,
    pub save: bool,
    pub cancel: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ActionToolbarConfig {
    pub show_copy: bool,
    pub show_save: bool,
    /// Current state of the output-border toggle. Drives the selected
    /// chrome on the Border button.
    pub border_active: bool,
}

/// Vertical action bar pinned to the right of the region (or below it if
/// the right edge has no room). Holds the confirm-flow buttons so the
/// main toolbar stays focused on tool / colour selection.
pub(crate) fn draw_action_toolbar(
    ctx: &egui::Context,
    region: sss_capture::Rect,
    monitor_origin: Pos2,
    monitor_size: Vec2,
    main_toolbar_rect: Option<Rect>,
    chrome: &ChromeColors,
    cfg: ActionToolbarConfig,
    icons: &mut IconCache,
) -> (ActionToolbarOutcome, Rect) {
    let mut out = ActionToolbarOutcome::default();

    // Build the button list.
    let mut entries: Vec<(crate::icons::ToolbarIcon, &str)> = vec![
        (crate::icons::ToolbarIcon::Undo, "undo"),
        (crate::icons::ToolbarIcon::Redo, "redo"),
        (crate::icons::ToolbarIcon::Border, "border"),
        (crate::icons::ToolbarIcon::Clear, "clear"),
        (crate::icons::ToolbarIcon::Confirm, "confirm"),
    ];
    if cfg.show_copy {
        entries.push((crate::icons::ToolbarIcon::Copy, "copy"));
    }
    if cfg.show_save {
        entries.push((crate::icons::ToolbarIcon::Save, "save"));
    }
    entries.push((crate::icons::ToolbarIcon::Cancel, "cancel"));

    let n = entries.len() as f32;
    let total_w = TB_PAD_X * 2.0 + TB_BTN;
    let total_h = TB_PAD_Y * 2.0 + n * TB_BTN + (n - 1.0).max(0.0) * TB_GAP;

    let rx = region.x() as f32 - monitor_origin.x;
    let ry = region.y() as f32 - monitor_origin.y;
    let rw = region.width() as f32;
    let rh = region.height() as f32;

    // Prefer right side; fall back to left; clamp inside monitor.
    let main_right = main_toolbar_rect.map(|r| r.max.x).unwrap_or(0.0);
    let main_left = main_toolbar_rect.map(|r| r.min.x).unwrap_or(monitor_size.x);
    let right_edge = (rx + rw).max(main_right) + TB_GAP_FROM_REGION;
    let left_edge = rx.min(main_left) - total_w - TB_GAP_FROM_REGION;
    let mut tb_x = if right_edge + total_w <= monitor_size.x - 8.0 {
        right_edge
    } else if left_edge >= 8.0 {
        left_edge
    } else {
        monitor_size.x - total_w - 8.0
    };
    tb_x = tb_x.clamp(8.0, (monitor_size.x - total_w - 8.0).max(8.0));

    let mut tb_y = ry + (rh - total_h) / 2.0;
    tb_y = tb_y.clamp(8.0, (monitor_size.y - total_h - 8.0).max(8.0));

    // Avoid overlapping the main toolbar by nudging vertically.
    if let Some(main) = main_toolbar_rect {
        let candidate =
            Rect::from_min_size(Pos2::new(tb_x, tb_y), Vec2::new(total_w, total_h));
        if candidate.intersects(main) {
            let above_room = main.min.y - 8.0;
            let below_room = monitor_size.y - main.max.y - 8.0;
            tb_y = if below_room >= total_h && below_room >= above_room {
                (main.max.y + 4.0).min(monitor_size.y - total_h - 8.0)
            } else {
                (main.min.y - total_h - 4.0).max(8.0)
            };
        }
    }

    let bar_rect = Rect::from_min_size(Pos2::new(tb_x, tb_y), Vec2::new(total_w, total_h));
    let bg = Color32::from_rgb(
        chrome.toolbar_bg.0[0],
        chrome.toolbar_bg.0[1],
        chrome.toolbar_bg.0[2],
    );
    let border = Color32::from_rgb(
        chrome.toolbar_border.0[0],
        chrome.toolbar_border.0[1],
        chrome.toolbar_border.0[2],
    );
    let fg = Color32::from_rgb(
        chrome.toolbar_fg.0[0],
        chrome.toolbar_fg.0[1],
        chrome.toolbar_fg.0[2],
    );

    egui::Area::new(egui::Id::new("sss::action_toolbar"))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(tb_x, tb_y))
        .interactable(true)
        .show(ctx, |ui| {
            // Paint the bg directly — *don't* allocate a hover sense over
            // the whole bar, otherwise egui treats the bar as the
            // top-of-z-order widget and the per-button `interact()` calls
            // never observe a click.
            let painter = ui.painter();
            painter.rect_filled(bar_rect, TB_RADIUS, bg);
            painter.rect_stroke(
                bar_rect,
                TB_RADIUS,
                Stroke::new(1.0, border),
                egui::StrokeKind::Inside,
            );

            let cursor_x = bar_rect.min.x + TB_PAD_X;
            let mut cursor_y = bar_rect.min.y + TB_PAD_Y;
            for (icon, name) in entries.iter() {
                let btn_rect =
                    Rect::from_min_size(Pos2::new(cursor_x, cursor_y), Vec2::splat(TB_BTN));
                let resp = ui.interact(
                    btn_rect,
                    egui::Id::new(("sss::act_btn", *name)),
                    Sense::click(),
                );
                let is_active = matches!(icon, crate::icons::ToolbarIcon::Border)
                    && cfg.border_active;
                let bgc = if is_active {
                    Color32::from_rgb(
                        chrome.button_active_bg.0[0],
                        chrome.button_active_bg.0[1],
                        chrome.button_active_bg.0[2],
                    )
                } else if resp.hovered() {
                    Color32::from_rgb(
                        chrome.button_bg.0[0].saturating_add(20),
                        chrome.button_bg.0[1].saturating_add(20),
                        chrome.button_bg.0[2].saturating_add(20),
                    )
                } else {
                    Color32::from_rgb(
                        chrome.button_bg.0[0],
                        chrome.button_bg.0[1],
                        chrome.button_bg.0[2],
                    )
                };
                painter.rect_filled(btn_rect, 4.0, bgc);
                if is_active {
                    painter.rect_stroke(
                        btn_rect,
                        4.0,
                        Stroke::new(
                            1.0,
                            Color32::from_rgb(
                                chrome.button_active_border.0[0],
                                chrome.button_active_border.0[1],
                                chrome.button_active_border.0[2],
                            ),
                        ),
                        egui::StrokeKind::Inside,
                    );
                }
                let tint = match icon {
                    crate::icons::ToolbarIcon::Cancel => [220, 90, 90],
                    crate::icons::ToolbarIcon::Confirm => [110, 200, 130],
                    _ => [fg.r(), fg.g(), fg.b()],
                };
                if let Some(tex) = icons.get(ui.ctx(), *icon, tint) {
                    let icon_rect = Rect::from_center_size(btn_rect.center(), Vec2::splat(ICON_PIX));
                    painter.image(
                        tex.id(),
                        icon_rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                if resp.clicked() {
                    match icon {
                        crate::icons::ToolbarIcon::Undo => out.undo = true,
                        crate::icons::ToolbarIcon::Redo => out.redo = true,
                        crate::icons::ToolbarIcon::Border => out.border_toggle = true,
                        crate::icons::ToolbarIcon::Clear => out.clear_all = true,
                        crate::icons::ToolbarIcon::Confirm => out.confirm = true,
                        crate::icons::ToolbarIcon::Copy => out.copy = true,
                        crate::icons::ToolbarIcon::Save => out.save = true,
                        crate::icons::ToolbarIcon::Cancel => out.cancel = true,
                        _ => {}
                    }
                }
                cursor_y += TB_BTN + TB_GAP;
            }
            // Reserve the bar's full rect so the Area's interactable
            // bounds match what we drew (otherwise the Area only sees the
            // last button's rect).
            ui.allocate_rect(bar_rect, Sense::hover());
        });

    (out, bar_rect)
}

pub(crate) fn draw_gizmos(
    painter: &egui::Painter,
    bounds: sss_capture::Rect,
    screen_offset: Pos2,
    chrome: &ChromeColors,
) {
    let x0 = bounds.x() as f32 - screen_offset.x;
    let y0 = bounds.y() as f32 - screen_offset.y;
    let x1 = x0 + bounds.width() as f32;
    let y1 = y0 + bounds.height() as f32;
    let rect = Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1));

    let accent = Color32::from_rgb(
        chrome.button_active_border.0[0],
        chrome.button_active_border.0[1],
        chrome.button_active_border.0[2],
    );
    let fill = Color32::from_rgb(
        chrome.button_active_bg.0[0],
        chrome.button_active_bg.0[1],
        chrome.button_active_bg.0[2],
    );

    // Dashed outline so the selection reads as a marching-ants box and
    // doesn't get confused with a solid stroke the user might have drawn.
    let outline = Stroke::new(1.5, accent);
    let edges = [
        (rect.min, Pos2::new(rect.max.x, rect.min.y)),
        (Pos2::new(rect.max.x, rect.min.y), rect.max),
        (rect.max, Pos2::new(rect.min.x, rect.max.y)),
        (Pos2::new(rect.min.x, rect.max.y), rect.min),
    ];
    for (a, b) in edges {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let ux = dx / len;
        let uy = dy / len;
        let step = 8.0 + 5.0;
        let mut t = 0.0;
        while t < len {
            let t1 = (t + 8.0).min(len);
            painter.line_segment(
                [
                    Pos2::new(a.x + ux * t, a.y + uy * t),
                    Pos2::new(a.x + ux * t1, a.y + uy * t1),
                ],
                outline,
            );
            t += step;
        }
    }

    let br = Pos2::new(x1, y1);
    painter.circle_filled(br, GIZMO_HANDLE_R, fill);
    painter.circle_stroke(br, GIZMO_HANDLE_R, Stroke::new(1.5, accent));

    let mid_top = Pos2::new((x0 + x1) * 0.5, y0);
    let rot = Pos2::new(mid_top.x, y0 - GIZMO_ROTATE_OFFSET);
    painter.line_segment([mid_top, rot], Stroke::new(1.0, accent));
    painter.circle_filled(rot, GIZMO_HANDLE_R, fill);
    painter.circle_stroke(rot, GIZMO_HANDLE_R, Stroke::new(1.5, accent));
}

// ============================================================================
// Snap helpers
// ============================================================================

const SNAP_TOL: f32 = 8.0;

pub(crate) fn snap_point(canvas: &Canvas, p: crate::geometry::FPoint, step: f32) -> crate::geometry::FPoint {
    let step = step.max(2.0);
    let mut best = crate::geometry::FPoint::new(
        (p.x / step).round() * step,
        (p.y / step).round() * step,
    );
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

fn shape_vertices(shape: &crate::shape::Shape) -> Vec<crate::geometry::FPoint> {
    use crate::geometry::FPoint;
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

/// Sample the pixel at `pointer_global` from the eager-captured image and
/// return its RGB. `monitors_bb_origin` is the top-left of the bounding box
/// of all monitors (the local origin of the capture).
pub(crate) fn sample_capture(
    initial: &sss_capture::Image,
    monitors_bb_origin: (i32, i32),
    pointer_global: (i32, i32),
) -> Option<SssColor> {
    let bg = initial.as_rgba();
    let bx = (pointer_global.0 - monitors_bb_origin.0).max(0) as u32;
    let by = (pointer_global.1 - monitors_bb_origin.1).max(0) as u32;
    if bx >= bg.width() || by >= bg.height() {
        return None;
    }
    let px = bg.get_pixel(bx, by);
    Some(SssColor::rgb(px.0[0], px.0[1], px.0[2]))
}

fn apply_tool_color(t: &mut Tool, c: SssColor) {
    match t {
        Tool::Brush(b)
        | Tool::Line(b)
        | Tool::Arrow(b)
        | Tool::Rectangle(b)
        | Tool::Ellipse(b)
        | Tool::Polygon(b) => b.color = c,
        Tool::Step(s) => s.fill = c,
        Tool::Text(t) => t.color = c,
        Tool::Pointer | Tool::Eraser { .. } | Tool::BlurRect { .. } => {}
    }
}
