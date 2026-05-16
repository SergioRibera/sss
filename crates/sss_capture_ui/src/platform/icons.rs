//! SVG icon loader + cache for the wayland layer-shell toolbar.
//!
//! Each [`ToolbarIcon`](super::super::ToolbarIcon) variant has a matching
//! SVG file under `crates/sss_capture_ui/assets/icons/`. They are embedded
//! at compile time, parsed + rasterised once via [`resvg`], and cached as
//! BGRA pixel buffers ready to blit onto the SHM surface.
//!
//! Empty placeholder SVGs ship with the crate; until you replace them
//! with real artwork, the toolbar falls back to the built-in CPU-drawn
//! icon. Each parsed icon's `currentColor` is resolved against the
//! foreground colour the caller supplies so the same SVG renders nicely
//! against both light and dark backgrounds.

use std::sync::OnceLock;

use super::wayland_layer::ToolbarIcon;

const ICON_SIZE: u32 = 20;

macro_rules! icon_bytes {
    ($name:literal) => {
        include_bytes!(concat!("../../assets/icons/", $name, ".svg")) as &'static [u8]
    };
}

fn raw_svg(icon: ToolbarIcon) -> &'static [u8] {
    match icon {
        ToolbarIcon::Pointer => icon_bytes!("pointer"),
        ToolbarIcon::Brush => icon_bytes!("brush"),
        ToolbarIcon::Line => icon_bytes!("line"),
        ToolbarIcon::Arrow => icon_bytes!("arrow"),
        ToolbarIcon::Rectangle => icon_bytes!("rectangle"),
        ToolbarIcon::RectangleFilled => icon_bytes!("rectangle_filled"),
        ToolbarIcon::Ellipse => icon_bytes!("ellipse"),
        ToolbarIcon::EllipseFilled => icon_bytes!("ellipse_filled"),
        ToolbarIcon::Blur => icon_bytes!("blur"),
        ToolbarIcon::Eraser => icon_bytes!("eraser"),
        ToolbarIcon::Step => icon_bytes!("step"),
        ToolbarIcon::Text => icon_bytes!("text"),
        ToolbarIcon::Polygon => icon_bytes!("polygon"),
        ToolbarIcon::PolygonFilled => icon_bytes!("polygon_filled"),
        ToolbarIcon::Undo => icon_bytes!("undo"),
        ToolbarIcon::Redo => icon_bytes!("redo"),
        ToolbarIcon::Cancel => icon_bytes!("cancel"),
        ToolbarIcon::Confirm => icon_bytes!("confirm"),
        ToolbarIcon::Copy => icon_bytes!("copy"),
        ToolbarIcon::Save => icon_bytes!("save"),
        ToolbarIcon::ColorSwatch => icon_bytes!("color_swatch"),
        ToolbarIcon::Clear => icon_bytes!("clear"),
        ToolbarIcon::Pipette => icon_bytes!("pipette"),
        ToolbarIcon::Snap => icon_bytes!("snap"),
        ToolbarIcon::Magnifier => icon_bytes!("magnifier"),
        ToolbarIcon::Raise => icon_bytes!("raise"),
        ToolbarIcon::Lower => icon_bytes!("lower"),
        ToolbarIcon::Trash => icon_bytes!("trash"),
        ToolbarIcon::GizmoScale => icon_bytes!("gizmo_scale"),
        ToolbarIcon::GizmoRotate => icon_bytes!("gizmo_rotate"),
    }
}

/// A rasterised icon, ready to blit into the SHM buffer.
pub(crate) struct RasterIcon {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA bytes, row-major.
    pub rgba: Vec<u8>,
}

static CACHE: OnceLock<
    std::sync::Mutex<std::collections::HashMap<(ToolbarIcon, [u8; 3]), Option<RasterIcon>>>,
> = OnceLock::new();

/// Returns the rasterised icon for the given `kind` painted in `rgb`, or
/// `None` when the SVG is empty / unparseable (caller should fall back to
/// the built-in CPU drawer).
pub(crate) fn rasterise(kind: ToolbarIcon, rgb: [u8; 3]) -> Option<&'static RasterIcon> {
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(Default::default()));
    let mut guard = cache.lock().unwrap();
    // We unsafely promote the &RasterIcon to 'static via leaking the box —
    // entries are never removed and the cache lives for the lifetime of the
    // program. That gives us a stable address callers can reuse.
    if let Some(entry) = guard.get(&(kind, rgb)) {
        return entry.as_ref().map(|r| {
            // SAFETY: nothing in the map is ever removed.
            unsafe { &*(r as *const RasterIcon) }
        });
    }
    let raster = rasterise_uncached(kind, rgb);
    let inserted = guard.entry((kind, rgb)).or_insert(raster);
    inserted
        .as_ref()
        .map(|r| unsafe { &*(r as *const RasterIcon) })
}

fn rasterise_uncached(kind: ToolbarIcon, rgb: [u8; 3]) -> Option<RasterIcon> {
    let bytes = raw_svg(kind);
    if !looks_useful(bytes) {
        return None;
    }
    // Resolve `currentColor` to the foreground RGB by rewriting the SVG
    // bytes before parsing — `usvg::Options` doesn't expose a hook for
    // the `currentColor` value in v0.43.
    let recoloured;
    let bytes = if let Ok(s) = std::str::from_utf8(bytes) {
        if s.contains("currentColor") {
            let css = format!("rgb({},{},{})", rgb[0], rgb[1], rgb[2]);
            recoloured = s.replace("currentColor", &css).into_bytes();
            recoloured.as_slice()
        } else {
            bytes
        }
    } else {
        bytes
    };
    let opt = usvg::Options::default();
    let tree = match usvg::Tree::from_data(bytes, &opt) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(?kind, error = %e, "icon SVG parse failed; falling back to CPU drawer");
            return None;
        }
    };
    let scale = ICON_SIZE as f32 / tree.size().width().max(tree.size().height());
    let mut pixmap = tiny_skia::Pixmap::new(ICON_SIZE, ICON_SIZE)?;
    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(RasterIcon {
        width: pixmap.width(),
        height: pixmap.height(),
        rgba: pixmap.data().to_vec(),
    })
}

/// Returns true when the SVG looks like it has at least one drawable
/// element. Empty placeholders (just an `<svg></svg>` wrapper) come back
/// as `false`, which signals the caller to use the built-in glyph.
fn looks_useful(bytes: &[u8]) -> bool {
    let s = std::str::from_utf8(bytes).unwrap_or("");
    const SHAPE_TAGS: &[&str] = &[
        "<path ",
        "<line ",
        "<rect ",
        "<circle ",
        "<ellipse ",
        "<polygon ",
        "<polyline ",
        "<text ",
        "<g ",
        "<use ",
    ];
    SHAPE_TAGS.iter().any(|t| s.contains(t))
}
