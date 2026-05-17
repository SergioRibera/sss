//! Color primitive used by the toolbar and shape styles.

/// Premultiplied RGBA color, 8 bits per channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const TRANSPARENT: Self = Self([0, 0, 0, 0]);
    pub const BLACK: Self = Self([0, 0, 0, 255]);
    pub const WHITE: Self = Self([255, 255, 255, 255]);
    pub const RED: Self = Self([220, 50, 47, 255]);
    pub const ORANGE: Self = Self([255, 140, 0, 255]);
    pub const YELLOW: Self = Self([240, 200, 0, 255]);
    pub const GREEN: Self = Self([50, 180, 80, 255]);
    pub const BLUE: Self = Self([60, 120, 230, 255]);
    pub const PURPLE: Self = Self([170, 90, 230, 255]);
    pub const SHADOW: Self = Self([0, 0, 0, 140]);
    pub const ACCENT: Self = Self([90, 170, 255, 255]);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b, 255])
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self([r, g, b, a])
    }

    /// RGB triplet with the alpha channel dropped.
    pub const fn to_rgb(self) -> [u8; 3] {
        [self.0[0], self.0[1], self.0[2]]
    }

    /// Default toolbar palette.
    pub fn palette() -> &'static [Color] {
        &[
            Color::RED,
            Color::ORANGE,
            Color::YELLOW,
            Color::GREEN,
            Color::BLUE,
            Color::PURPLE,
            Color::BLACK,
            Color::WHITE,
        ]
    }

    pub fn with_alpha(mut self, a: u8) -> Self {
        self.0[3] = a;
        self
    }

    /// Parse `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA` (`#` is optional).
    pub fn parse_hex(s: &str) -> Result<Self, String> {
        sss_core::color::parse_hex(s)
            .map(Self)
            .map_err(|e| e.to_string())
    }

    /// Hex form (round-trips through `parse_hex`).
    pub fn to_hex(self) -> String {
        sss_core::color::to_hex(self.0)
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::RED
    }
}

impl From<[u8; 4]> for Color {
    fn from(v: [u8; 4]) -> Self {
        Self(v)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse_hex(&s).map_err(serde::de::Error::custom)
    }
}
