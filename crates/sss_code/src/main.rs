#![allow(clippy::expect_fun_call)]
use std::borrow::Cow;

use config::get_config;
use img::ImageCode;
use sss_lib::generate_image;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use theme::{list_themes, load_theme, theme_from_vim};

mod config;
mod error;
mod img;
mod theme;
mod utils;

// wl-paste | cargo run -p sss_code -- -t InspiredGitHub -e rs --lines 2..10 --vim-theme "Normal,#dab997,#262626,,;LineNr,#949494,#262626,,;Visual,,#4e4e4e,,;Cursor,#262626,#dab997;CursorLine,,#3a3a3a,,;Search,#3a3a3a,ffaf00;SpellBad,#d75f5f,,undercurl,;Title,#83adad,,,;MatchParen,,#8a8a8a,,;IdentBlanklineChar,#4e4e4e,,,;Number,#ff8700,,,;Character,#d75f5f,,,;String,#afaf00,,,;Constant,#ff8700,,,;Identifier,#d75f5f,,,;Keyword,#d485ad,,,;Comment,#8a8a8a,,,;Operator,#d485ad,,,;Statement,#d75f5f,,,;Type,#ffaf00,,,;StorageClass,#ffaf00,,,;Function,#83adad,,," -
fn main() {
    let (config, g_config) = get_config();

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
