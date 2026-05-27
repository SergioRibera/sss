//! Cross-platform cursor decision logic shared by the legacy layer-shell
//! driver and the new winit / egui driver.

use crate::canvas::{Canvas, RegionHandle};
use crate::geometry::FPoint;
use crate::mode::SelectorMode;

/// Standard cursor names we know how to ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CursorName {
    Default,
    Crosshair,
    Move,
    NwSeResize,
    NeSwResize,
    NsResize,
    EwResize,
}

/// Decide which cursor should show, based on the canvas state, pointer
/// position, and a few caller-supplied hints (pointer over toolbar /
/// over a gizmo handle / current selector mode).
pub fn desired_cursor_ext(
    canvas: &Canvas,
    pointer: FPoint,
    pointer_on_toolbar: bool,
    pointer_on_gizmo: bool,
    mode: SelectorMode,
) -> CursorName {
    if pointer_on_toolbar {
        return CursorName::Default;
    }
    if pointer_on_gizmo {
        return CursorName::NwSeResize;
    }
    if matches!(canvas.active_tool, crate::tool::Tool::Pointer) {
        for s in canvas.shapes().iter().rev() {
            if s.contains(pointer) {
                return CursorName::Move;
            }
        }
        if let Some(region) = canvas.region() {
            if let Some(handle) = crate::canvas::pointer_handle_pub(&region, pointer) {
                return match handle {
                    RegionHandle::NW | RegionHandle::SE => CursorName::NwSeResize,
                    RegionHandle::NE | RegionHandle::SW => CursorName::NeSwResize,
                    RegionHandle::N | RegionHandle::S => CursorName::NsResize,
                    RegionHandle::E | RegionHandle::W => CursorName::EwResize,
                };
            }
        }
        return CursorName::Crosshair;
    }
    match mode {
        SelectorMode::Monitor | SelectorMode::Window => CursorName::Default,
        SelectorMode::Area | SelectorMode::AnyOf => CursorName::Crosshair,
    }
}
