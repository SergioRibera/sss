//! When does the actual capture happen?

/// Capture timing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureTrigger {
    /// Take the screenshot *before* showing the overlay; the user draws on
    /// top of the static image. Robust on every platform and the default.
    Eager,
    /// Show the overlay over the live desktop and only call into
    /// [`sss_capture::Capturer`] once the user confirms with `confirm`.
    Lazy {
        /// Keybind that confirms the selection (default: Enter).
        confirm: KeyChord,
        /// Optional alternative confirm (e.g. a toolbar button click).
        confirm_button_label: Option<String>,
    },
}

impl Default for CaptureTrigger {
    fn default() -> Self {
        CaptureTrigger::Eager
    }
}

/// A key combination — modifier flags + a logical key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    pub key: KeyBind,
}

impl KeyChord {
    pub const fn key(k: KeyBind) -> Self {
        Self {
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
            key: k,
        }
    }

    pub const fn ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }
    pub const fn shift(mut self) -> Self {
        self.shift = true;
        self
    }
    pub const fn alt(mut self) -> Self {
        self.alt = true;
        self
    }
    pub const fn meta(mut self) -> Self {
        self.meta = true;
        self
    }
}

/// Logical key the overlay cares about. Maps directly to the relevant
/// `winit::keyboard::Key` variants — anything not listed here is ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyBind {
    Enter,
    Escape,
    Space,
    Tab,
    Delete,
    Backspace,
    /// A printable character. Stored lowercase.
    Char(char),
    F(u8),
}

impl KeyBind {
    pub const ENTER: KeyChord = KeyChord::key(KeyBind::Enter);
    pub const ESC: KeyChord = KeyChord::key(KeyBind::Escape);
}
