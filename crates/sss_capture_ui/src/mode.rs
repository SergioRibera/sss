//! Selector modes.

/// Mode the overlay is in when it opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum SelectorMode {
    #[default]
    Area,
    Monitor,
    Window,
    /// User toggles between Area / Monitor / Window via the toolbar.
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
