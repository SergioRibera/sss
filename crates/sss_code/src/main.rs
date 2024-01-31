#![allow(clippy::expect_fun_call)]
use std::borrow::Cow;

use sss_code::config::get_config;
use sss_code::ImageCode;
use sss_code::{list_themes, load_theme, theme_from_vim};
use sss_lib::generate_image;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

fn main() {
    let (config, mut g_config) = get_config();

    let mut ss = SyntaxSet::load_defaults_newlines();
    let themes = ThemeSet::load_defaults();

    if let Some(dir) = &config.extra_syntaxes {
        let mut builder = ss.into_builder();
        builder.add_from_folder(dir, true).unwrap();
        ss = builder.build();
    }

    if config.list_themes {
        list_themes(&themes);
        return;
    }

    if config.list_file_types {
        list_file_types(&ss);
        return;
    }

    let content = config.content.clone().unwrap().contents().unwrap();
    let syntax = if let Some(ext) = &config.extension {
        ss.find_syntax_by_extension(ext)
            .expect(&format!("Extension not found: {ext}"))
    } else {
        ss.find_syntax_by_first_line(content.split('\n').next().unwrap())
            .expect("Extension not found by code")
    };

    let theme = if let Some(vim_theme) = &config.vim_theme {
        Cow::Owned(theme_from_vim(vim_theme))
    } else {
        let theme = config
            .theme
            .clone()
            .unwrap_or("base16-ocean.dark".to_string());
        themes
            .themes
            .get(&theme)
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(load_theme(&theme, true)))
    };

    if theme.settings.background.is_some()
        && g_config.colors.windows_background
            == sss_lib::Background::Solid(sss_lib::image::Rgba([0x42, 0x87, 0xf5, 255]))
    {
        g_config.colors.windows_background = theme
            .settings
            .background
            .map(|c| sss_lib::Background::Solid(sss_lib::image::Rgba([c.r, c.g, c.b, c.a])))
            .unwrap();
    }

    generate_image(
        g_config.clone(),
        ImageCode {
            config,
            syntax,
            theme,
            lib_config: g_config.clone(),
            syntax_set: &ss,
            content: &content,
            font: g_config.fonts,
        },
    );
}

fn list_file_types(ss: &SyntaxSet) {
    for s in ss.syntaxes() {
        println!("- {} (.{})", s.name, s.file_extensions.join(", ."));
    }
}
