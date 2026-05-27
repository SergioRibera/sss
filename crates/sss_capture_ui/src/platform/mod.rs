//! Platform driver dispatch — single winit/egui driver across every OS.
//! Wayland gets native `zwlr_layer_shell_v1` support via the patched
//! `winit-wayland` backend; X11 and other backends fall back to a
//! borderless fullscreen toplevel.

mod driver;

pub(crate) fn run(
    sel: crate::selector::Selector,
) -> Result<crate::selector::Selection, crate::selector::SelectorError> {
    driver::run(sel)
}
