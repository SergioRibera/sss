//! Platform entry point. GPUI handles X11, Wayland (xdg-shell +
//! wlr-layer-shell when the compositor exposes it), and Cocoa, so a single
//! driver covers every supported target.

mod driver;

pub(crate) fn run(
    sel: crate::selector::Selector,
) -> Result<crate::selector::Selection, crate::selector::SelectorError> {
    driver::run(sel)
}
