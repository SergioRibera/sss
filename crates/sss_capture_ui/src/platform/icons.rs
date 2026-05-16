//! SVG icon loader and rasterisation cache for the layer-shell toolbar.

use std::sync::OnceLock;

use egui::ahash::HashMap;
use egui::mutex::Mutex;

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

pub(crate) struct RasterIcon {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA, row-major.
    pub rgba: Vec<u8>,
}

type ToolbarIconCache = HashMap<(ToolbarIcon, [u8; 3]), Option<RasterIcon>>;
static CACHE: OnceLock<Mutex<ToolbarIconCache>> = OnceLock::new();

/// Returns the rasterised icon, or `None` when the SVG is empty/unparseable.
pub(crate) fn rasterise(kind: ToolbarIcon, rgb: [u8; 3]) -> Option<&'static RasterIcon> {
    let cache = CACHE.get_or_init(|| Mutex::new(Default::default()));
    let mut guard = cache.lock();
    if let Some(entry) = guard.get(&(kind, rgb)) {
        return entry.as_ref().map(|r| {
            // SAFETY: cache entries are never removed; addresses are stable.
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
    // usvg 0.43 has no hook for the `currentColor` value, so rewrite it in
    // the source bytes before parsing.
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
