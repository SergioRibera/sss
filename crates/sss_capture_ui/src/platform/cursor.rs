//! Per-context cursor management for the wayland layer-shell overlay.
//!
//! Wayland clients are responsible for picking the cursor pixmap their
//! surfaces show; nothing about the layer-shell protocol changes that.
//! This module loads the user's system cursor theme via `wayland-cursor`
//! and lets the driver swap between standard X11 cursor names —
//! `crosshair`, `move`, `nwse-resize`, … — based on the canvas / toolbar
//! state.
//!
//! Cursors are loaded lazily (the first `set` for a given name parses the
//! image), then cached as a `wl_surface` with the cursor image attached.
//! `wl_pointer.set_cursor` is only emitted when the requested name differs
//! from the current one, so this is cheap to call on every motion event.

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
    /// X11-style cursor name. Modern themes (Adwaita, Bibata, Catppuccin)
    /// all ship these; older ones may need a fallback.
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

    /// Legacy fallback name used when the theme doesn't ship the modern
    /// one. e.g. `nwse-resize` → `top_left_corner`.
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

/// Variant that also takes a `pointer_on_gizmo` flag so the driver can
/// swap to a resize cursor while the pointer hovers the transform-gizmo
/// handle in the selection decoration.
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
    // Pointer tool: the cursor reflects what would happen if the user
    // clicked *right now*. We don't show a Move cursor just because the
    // pointer is inside the selection rectangle — that mislead the user
    // into thinking they'd grab the region instead of starting a new
    // drag.
    if matches!(canvas.active_tool, crate::tool::Tool::Pointer) {
        // Over a committed shape → move.
        for s in canvas.shapes().iter().rev() {
            if s.contains(pointer) {
                return CursorName::Move;
            }
        }
        // Over a region resize handle → matching resize cursor.
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
        // Otherwise the user is in empty space — clicking will start a
        // fresh rubber-band, so show the create-area cursor.
        return CursorName::Crosshair;
    }
    // Drawing tools.
    match mode {
        SelectorMode::Monitor | SelectorMode::Window => CursorName::Default,
        SelectorMode::Area | SelectorMode::AnyOf => CursorName::Crosshair,
    }
}

/// Per-pointer cursor cache.
pub(crate) struct CursorContext {
    theme: CursorTheme,
    /// `wl_surface` plus its hotspot for each cached cursor name.
    surfaces: HashMap<CursorName, (WlSurface, i32, i32)>,
    /// What the pointer is currently showing — used to skip redundant
    /// `set_cursor` requests on every motion event.
    current: Option<CursorName>,
    /// Most recent `wl_pointer.enter` serial; the protocol requires it on
    /// every subsequent `set_cursor` call.
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

    /// Force the next `apply` to actually send a `set_cursor` even if the
    /// cached cursor name hasn't changed. Called on pointer-enter so the
    /// compositor's freshly-attached pointer image gets refreshed.
    pub fn invalidate(&mut self) {
        self.current = None;
    }

    /// Make the pointer show `name`. No-op when already showing it.
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
            // `CursorImageBuffer` derefs to a `WlBuffer`.
            surface.attach(Some(&*image), 0, 0);
            surface.damage_buffer(0, 0, w as i32, h as i32);
            surface.commit();
            self.surfaces
                .insert(name, (surface, hot_x as i32, hot_y as i32));
        }
        self.surfaces.get(&name)
    }
}
