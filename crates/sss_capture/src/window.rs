//! Window descriptor.

use std::fmt;

use crate::geometry::Rect;
use crate::monitor::MonitorId;

/// Opaque, backend-supplied window identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub(crate) u64);

impl WindowId {
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "w#{}", self.0)
    }
}

/// A top-level window known to the platform.
#[derive(Clone, Debug)]
pub struct Window {
    pub(crate) id: WindowId,
    pub(crate) title: String,
    pub(crate) app_name: String,
    pub(crate) bounds: Rect,
    pub(crate) monitor: Option<MonitorId>,
    pub(crate) is_minimized: bool,
    pub(crate) is_maximized: bool,
    pub(crate) is_focused: bool,
}

impl Window {
    #[inline]
    pub fn id(&self) -> WindowId {
        self.id
    }
    #[inline]
    pub fn title(&self) -> &str {
        &self.title
    }
    #[inline]
    pub fn app_name(&self) -> &str {
        &self.app_name
    }
    #[inline]
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
    #[inline]
    pub fn monitor(&self) -> Option<MonitorId> {
        self.monitor
    }
    #[inline]
    pub fn is_minimized(&self) -> bool {
        self.is_minimized
    }
    #[inline]
    pub fn is_maximized(&self) -> bool {
        self.is_maximized
    }
    #[inline]
    pub fn is_focused(&self) -> bool {
        self.is_focused
    }
}

impl fmt::Display for Window {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {:?} ({:?}) {}",
            self.id, self.app_name, self.title, self.bounds
        )
    }
}

/// Search predicate accepted by [`crate::Capturer::find_window`].
#[derive(Clone, Debug, Default)]
pub struct WindowSearch {
    pub id: Option<WindowId>,
    pub title_contains: Option<String>,
    pub app_contains: Option<String>,
}

impl WindowSearch {
    pub fn by_id(id: WindowId) -> Self {
        Self {
            id: Some(id),
            ..Default::default()
        }
    }

    pub fn by_title(needle: impl Into<String>) -> Self {
        Self {
            title_contains: Some(needle.into()),
            ..Default::default()
        }
    }

    pub fn by_app(needle: impl Into<String>) -> Self {
        Self {
            app_contains: Some(needle.into()),
            ..Default::default()
        }
    }

    pub fn matches(&self, w: &Window) -> bool {
        if let Some(id) = self.id {
            return w.id == id;
        }
        if let Some(t) = &self.title_contains {
            if !w.title.to_lowercase().contains(&t.to_lowercase()) {
                return false;
            }
        }
        if let Some(a) = &self.app_contains {
            if !w.app_name.to_lowercase().contains(&a.to_lowercase()) {
                return false;
            }
        }
        self.id.is_some() || self.title_contains.is_some() || self.app_contains.is_some()
    }
}

impl From<WindowId> for WindowSearch {
    fn from(id: WindowId) -> Self {
        Self::by_id(id)
    }
}
impl From<u32> for WindowSearch {
    fn from(raw: u32) -> Self {
        Self::by_id(WindowId(raw as u64))
    }
}
impl From<u64> for WindowSearch {
    fn from(raw: u64) -> Self {
        Self::by_id(WindowId(raw))
    }
}
impl<'a> From<&'a str> for WindowSearch {
    fn from(s: &'a str) -> Self {
        Self::by_title(s)
    }
}
impl From<String> for WindowSearch {
    fn from(s: String) -> Self {
        Self::by_title(s)
    }
}
