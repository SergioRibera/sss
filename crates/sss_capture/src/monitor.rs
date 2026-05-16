//! Monitor / output descriptor.

use std::fmt;

use crate::geometry::{Rect, Rotation};

/// Opaque, backend-supplied monitor identifier. The wrapped value is whatever
/// the platform produces (RANDR output id, Wayland output `name`-FNV hash,
/// `HMONITOR`, CGDirectDisplayID). Treat it as an opaque token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MonitorId(pub(crate) u64);

impl MonitorId {
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for MonitorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// A physical display attached to the system.
///
/// All geometry is in **logical pixels** — the crate factors out
/// `scale_factor` and `rotation` before reporting `bounds`. The original
/// physical metrics are still available via [`Monitor::physical_size`] and
/// [`Monitor::scale_factor`].
#[derive(Clone, Debug)]
pub struct Monitor {
    pub(crate) id: MonitorId,
    pub(crate) name: String,
    pub(crate) bounds: Rect,
    pub(crate) physical_size: (u32, u32),
    pub(crate) scale_factor: f32,
    pub(crate) rotation: Rotation,
    pub(crate) refresh_rate: Option<f32>,
    pub(crate) is_primary: bool,
}

impl Monitor {
    #[inline]
    pub fn id(&self) -> MonitorId {
        self.id
    }
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Logical bounds on the virtual desktop.
    #[inline]
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
    /// Physical pixel size of the panel, before rotation is applied.
    #[inline]
    pub fn physical_size(&self) -> (u32, u32) {
        self.physical_size
    }
    #[inline]
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }
    #[inline]
    pub fn rotation(&self) -> Rotation {
        self.rotation
    }
    /// Refresh rate in Hertz, when the backend reports it.
    #[inline]
    pub fn refresh_rate(&self) -> Option<f32> {
        self.refresh_rate
    }
    #[inline]
    pub fn is_primary(&self) -> bool {
        self.is_primary
    }
}

impl fmt::Display for Monitor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hz = self
            .refresh_rate
            .map(|h| format!(" {h:.0}Hz"))
            .unwrap_or_default();
        write!(
            f,
            "{} {:?} ({}) {}× rot {:?}{}{}",
            self.id,
            self.name,
            self.bounds,
            self.scale_factor,
            self.rotation,
            hz,
            if self.is_primary { " primary" } else { "" }
        )
    }
}
