#![allow(clippy::expect_fun_call)]
use std::borrow::Cow;
use std::path::PathBuf;

use sss_code::config::get_config;
use sss_code::error::{CodeScreenshot, Configuration as ConfigurationError};
use sss_code::ImageCode;
use sss_code::{list_themes, load_theme, theme_from_vim};
use sss_lib::error::PrettyErrorWrapper;
use sss_lib::generate_image;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

const DEFAULT_SYNTAXSET: &[u8] = include_bytes!("../../../assets/syntaxes.bin");
const DEFAULT_THEMESET: &[u8] = include_bytes!("../../../assets/themes.bin");

fn main() -> Result<(), PrettyErrorWrapper<CodeScreenshot>> {
    let (config, mut g_config) = get_config()?;

    let cache_path = directories::BaseDirs::new()
        .ok_or(ConfigurationError::InvalidHome)?
        .cache_dir()
        .join("sss");

    let mut ss: SyntaxSet =
        if let Ok(ss) = syntect::dumps::from_dump_file(cache_path.join("syntaxes.bin")) {
            ss
        } else {
            syntect::dumps::from_binary(DEFAULT_SYNTAXSET)
        };
    let mut themes: ThemeSet =
        if let Ok(ts) = syntect::dumps::from_dump_file(cache_path.join("themes.bin")) {
            ts
        } else {
            syntect::dumps::from_binary(DEFAULT_THEMESET)
        };

    if let Some(dir) = &config.extra_syntaxes {
        let mut builder = ss.into_builder();
        builder.add_from_folder(dir, true)?;
        ss = builder.build();
        syntect::dumps::dump_to_file(&ss, cache_path.join("syntaxes.bin"))?;
    }

    if config.list_themes {
        list_themes(&themes);
        return Ok(());
    }

    if config.list_file_types {
        list_file_types(&ss);
        return Ok(());
    }

    // build cache of themes or syntaxes
    if let Some(from) = config.build_cache.as_ref() {
        let to = PathBuf::from(&g_config.output);

        themes.add_from_folder(from.join("themes"))?;
        let mut builder = ss.clone().into_builder();
        builder.add_from_folder(from.join("syntaxes"), true)?;
        ss = builder.build();

        syntect::dumps::dump_to_file(&themes, to.join("themes.bin"))?;
        syntect::dumps::dump_to_file(&ss, to.join("syntaxes.bin"))?;
        std::process::exit(0);
    }

    let content = config
        .content
        .clone()
        .expect("Cannot get content from args")
        .contents()
        .expect("Cannot get content to render");
    let syntax = if let Some(ext) = &config.extension {
        ss.find_syntax_by_extension(ext)?
    } else {
        ss.find_syntax_for_file(&content)?
            .expect(&format!("Extension not found from stdin or file"))
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
            .unwrap_or_else(|| Cow::Owned(load_theme(&theme, false)))
    };

    if theme.settings.background.is_some()
        && g_config.colors.windows_background
            == sss_lib::Background::Solid(sss_lib::image::Rgba([0x42, 0x87, 0xf5, 255]))
    {
        g_config.colors.windows_background = theme
            .settings
            .background
            .map(|c| sss_lib::Background::Solid(sss_lib::image::Rgba([c.r, c.g, c.b, c.a])))
            .ok_or(sss_code::error::ConfigurationError::ParamNotFound(
                "background".to_owned(),
            ))?
    }

    Ok(generate_image(
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
    )?)
}

fn list_file_types(ss: &SyntaxSet) {
    for s in ss.syntaxes() {
        println!("- {} (.{})", s.name, s.file_extensions.join(", ."));
    }
}
