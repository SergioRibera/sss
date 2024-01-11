use sss_lib::font::FontStyle;
use sss_lib::image::Rgba;
use syntect::highlighting::Color;
use syntect::highlighting::FontStyle as HiFontStyle;

pub fn color_to_rgba(c: Color) -> Rgba<u8> {
    Rgba([c.r, c.g, c.b, c.a])
}

pub fn fontstyle_from_syntect(style: HiFontStyle) -> FontStyle {
    if style.contains(HiFontStyle::BOLD) {
        if style.contains(HiFontStyle::ITALIC) {
            FontStyle::BoldItalic
        } else {
            FontStyle::Bold
        }
    } else if style.contains(HiFontStyle::ITALIC) {
        FontStyle::Italic
    } else {
        FontStyle::Regular
    }
}
