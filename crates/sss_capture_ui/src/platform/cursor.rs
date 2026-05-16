//! Cursor management for the wayland layer-shell overlay.

use std::collections::HashMap;

use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_pointer::WlPointer;
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, QueueHandle};
use wayland_cursor::CursorTheme;

use crate::canvas::{Canvas, RegionHandle};
use crate::geometry::FPoint;
use crate::mode::SelectorMode;

/// Standard cursor names we know how to ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CursorName {
    Default,
    Crosshair,
    Move,
    NwSeResize,
    NeSwResize,
    NsResize,
    EwResize,
}

impl CursorName {
    fn theme_name(self) -> &'static str {
        match self {
            CursorName::Default => "default",
            CursorName::Crosshair => "crosshair",
            CursorName::Move => "move",
            CursorName::NwSeResize => "nwse-resize",
            CursorName::NeSwResize => "nesw-resize",
            CursorName::NsResize => "ns-resize",
            CursorName::EwResize => "ew-resize",
        }
    }

    /// X11 legacy name for themes that don't ship the modern one.
    fn legacy_name(self) -> &'static str {
        match self {
            CursorName::Default => "left_ptr",
            CursorName::Crosshair => "cross",
            CursorName::Move => "fleur",
            CursorName::NwSeResize => "top_left_corner",
            CursorName::NeSwResize => "top_right_corner",
            CursorName::NsResize => "sb_v_double_arrow",
            CursorName::EwResize => "sb_h_double_arrow",
        }
    }
}

pub(crate) fn desired_cursor_ext(
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

pub(crate) struct CursorContext {
    theme: CursorTheme,
    surfaces: HashMap<CursorName, (WlSurface, i32, i32)>,
    current: Option<CursorName>,
    /// Most recent `wl_pointer.enter` serial; required by `set_cursor`.
    pub enter_serial: u32,
}

impl CursorContext {
    pub fn new(conn: &Connection, shm: WlShm, size: u32) -> Result<Self, String> {
        let theme =
            CursorTheme::load(conn, shm, size).map_err(|e| format!("CursorTheme::load: {e}"))?;
        Ok(Self {
            theme,
            surfaces: HashMap::new(),
            current: None,
            enter_serial: 0,
        })
    }

    pub fn invalidate(&mut self) {
        self.current = None;
    }

    pub fn apply(
        &mut self,
        pointer: &WlPointer,
        compositor: &WlCompositor,
        qh: &QueueHandle<crate::platform::wayland_layer::State>,
        name: CursorName,
    ) {
        if self.current == Some(name) {
            return;
        }
        let serial = self.enter_serial;
        let (surface, hot_x, hot_y) = match self.surface_for(name, compositor, qh) {
            Some(t) => (t.0.clone(), t.1, t.2),
            None => {
                tracing::trace!(?name, "no cursor available; leaving as-is");
                return;
            }
        };
        pointer.set_cursor(serial, Some(&surface), hot_x, hot_y);
        self.current = Some(name);
    }

    fn surface_for(
        &mut self,
        name: CursorName,
        compositor: &WlCompositor,
        qh: &QueueHandle<crate::platform::wayland_layer::State>,
    ) -> Option<&(WlSurface, i32, i32)> {
        if !self.surfaces.contains_key(&name) {
            let cursor = match self.theme.get_cursor(name.theme_name()) {
                Some(c) => Some(c),
                None => self.theme.get_cursor(name.legacy_name()),
            }?;
            if cursor.image_count() == 0 {
                return None;
            }
            let image = &cursor[0];
            let (hot_x, hot_y) = image.hotspot();
            let (w, h) = image.dimensions();
            let surface = compositor.create_surface(qh, ());
            surface.attach(Some(image), 0, 0);
            surface.damage_buffer(0, 0, w as i32, h as i32);
            surface.commit();
            self.surfaces
                .insert(name, (surface, hot_x as i32, hot_y as i32));
        }
        self.surfaces.get(&name)
    }
}
