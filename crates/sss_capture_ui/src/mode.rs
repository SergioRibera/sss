//! Selector modes — what is the user picking?

/// Mode the overlay is in when it opens.
///
/// `AnyOf` lets the user toggle between modes through the toolbar (default
/// when the toolbar is enabled). For headless / slurp-class flows pick a
/// single mode explicitly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum SelectorMode {
    /// Drag a rubber-band rectangle. Re-edit through the Pointer tool.
    #[default]
    Area,
    /// Click on a monitor to pick the whole thing.
    Monitor,
    /// Click on a top-level window. Each window is rendered as a thumbnail
    /// preview floating on top of its actual monitor — so the user can pick
    /// even when overlapping windows are hidden.
    Window,
    /// User toggles between Area / Monitor / Window via the toolbar tabs.
    AnyOf,
}

impl SelectorMode {
    pub fn label(self) -> &'static str {
        match self {
            SelectorMode::Area => "Area",
            SelectorMode::Monitor => "Monitor",
            SelectorMode::Window => "Window",
            SelectorMode::AnyOf => "Pick",
        }
    }
}
