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

pub fn process_token_text<F>(
    text: &str,
    space: char,
    tab_char: char,
    tab_size: usize,
    in_indent: &mut bool,
    mut indent_level: usize,
    get_indent_char: F,
) -> (String, usize)
where
    F: Fn(usize) -> char,
{
    let mut result = String::with_capacity(text.len());
    let mut consecutive_spaces = 0;

    for c in text.chars() {
        match c {
            ' ' => {
                consecutive_spaces += 1;
                if consecutive_spaces == tab_size {
                    if *in_indent {
                        result.push(get_indent_char(indent_level));
                        for _ in 0..tab_size - 1 {
                            result.push(tab_char);
                        }
                        indent_level += 1;
                    } else {
                        for _ in 0..tab_size {
                            result.push(space);
                        }
                    }
                    consecutive_spaces = 0;
                }
            }
            '\t' => {
                if *in_indent {
                    result.push(get_indent_char(indent_level));
                    for _ in 0..tab_size - 1 {
                        result.push(tab_char);
                    }
                    indent_level += 1;
                } else {
                    result.push(tab_char);
                }
                consecutive_spaces = 0;
            }
            _ => {
                if consecutive_spaces > 0 {
                    if *in_indent {
                        for _ in 0..consecutive_spaces {
                            result.push(tab_char);
                        }
                    } else {
                        for _ in 0..consecutive_spaces {
                            result.push(space);
                        }
                    }
                    consecutive_spaces = 0;
                }
                result.push(c);
                *in_indent = false; // Cambiamos a false después de encontrar un carácter no indentable
            }
        }
    }

    // Manejar espacios restantes al final del texto
    if consecutive_spaces > 0 {
        if *in_indent {
            for _ in 0..consecutive_spaces {
                result.push(tab_char);
            }
        } else {
            for _ in 0..consecutive_spaces {
                result.push(space);
            }
        }
    }

    (result, indent_level)
}
