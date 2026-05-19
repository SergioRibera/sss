//! Static asset source backing GPUI's `svg()` element. All icons are
//! embedded with `include_bytes!` so the binary stays self-contained.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

macro_rules! icon {
    ($name:literal) => {
        (
            concat!("icons/", $name, ".svg"),
            include_bytes!(concat!("../assets/icons/", $name, ".svg")) as &[u8],
        )
    };
}

const ICONS: &[(&str, &[u8])] = &[
    icon!("pointer"),
    icon!("brush"),
    icon!("line"),
    icon!("arrow"),
    icon!("rectangle"),
    icon!("ellipse"),
    icon!("polygon"),
    icon!("blur"),
    icon!("eraser"),
    icon!("step"),
    icon!("text"),
    icon!("pipette"),
    icon!("undo"),
    icon!("redo"),
    icon!("cancel"),
    icon!("confirm"),
    icon!("area"),
    icon!("monitor"),
    icon!("window"),
];

pub(crate) struct UiAssets;

impl AssetSource for UiAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS.iter().map(|(p, _)| SharedString::from(*p)).collect())
    }
}
