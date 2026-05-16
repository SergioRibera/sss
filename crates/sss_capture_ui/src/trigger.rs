//! Capture timing.

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum CaptureTrigger {
    /// Capture the screenshot before showing the overlay.
    #[default]
    Eager,
    /// Show the overlay over the live desktop and capture on confirm.
    Lazy {
        confirm: KeyChord,
        confirm_button_label: Option<String>,
    },
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

/// Logical key the overlay cares about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyBind {
    Enter,
    Escape,
    Space,
    Tab,
    Delete,
    Backspace,
    /// A printable character, stored lowercase.
    Char(char),
    F(u8),
}

impl KeyBind {
    pub const ENTER: KeyChord = KeyChord::key(KeyBind::Enter);
    pub const ESC: KeyChord = KeyChord::key(KeyBind::Escape);
}
