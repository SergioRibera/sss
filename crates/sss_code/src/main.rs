#![allow(clippy::expect_fun_call)]
use std::borrow::Cow;
use std::path::PathBuf;

use color_eyre::eyre::{ContextCompat, Report};
use sss_code::config::get_config;
use sss_code::error::Configuration as ConfigurationError;
use sss_code::ImageCode;
use sss_code::{list_themes, load_theme, theme_from_vim};
use sss_lib::generate_image;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use tracing_subscriber::EnvFilter;

const DEFAULT_SYNTAXSET: &[u8] = include_bytes!("../../../assets/syntaxes.bin");
const DEFAULT_THEMESET: &[u8] = include_bytes!("../../../assets/themes.bin");

fn main() -> Result<(), Report> {
    // install tracing
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new("off"))?)
        .with_timer(tracing_subscriber::fmt::time::Uptime::default())
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    // install color eyre
    color_eyre::config::HookBuilder::default()
        .issue_url(concat!(env!("CARGO_PKG_REPOSITORY"), "/issues/new"))
        .add_issue_metadata("version", env!("CARGO_PKG_VERSION"))
        .issue_filter(|kind| match kind {
            color_eyre::ErrorKind::NonRecoverable(_) => false,
            color_eyre::ErrorKind::Recoverable(_) => true,
        })
        .install()?;

    let (config, mut g_config) = get_config()?;
    tracing::trace!("Configs loaded");

    let cache_path = directories::BaseDirs::new()
        .ok_or(ConfigurationError::InvalidHome)?
        .cache_dir()
        .join("sss");

    let mut ss: SyntaxSet =
        if let Ok(ss) = syntect::dumps::from_dump_file(cache_path.join("syntaxes.bin")) {
            tracing::info!("Loading syntaxes from cache");
            ss
        } else {
            tracing::info!("Loading default syntaxes");
            syntect::dumps::from_binary(DEFAULT_SYNTAXSET)
        };
    let mut themes: ThemeSet =
        if let Ok(ts) = syntect::dumps::from_dump_file(cache_path.join("themes.bin")) {
            tracing::info!("Loading themes from cache");
            ts
        } else {
            tracing::info!("Loading default themes");
            syntect::dumps::from_binary(DEFAULT_THEMESET)
        };

    if let Some(dir) = &config.extra_syntaxes {
        if !dir.is_empty() {
            let mut builder = ss.into_builder();
            builder.add_from_folder(dir, true)?;
            ss = builder.build();
            tracing::debug!("Trying to load extra syntaxes");
            syntect::dumps::dump_to_file(&ss, cache_path.join("syntaxes.bin"))?;
        }
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
        tracing::trace!("Building cache of themes and syntaxes");

        themes.add_from_folder(from.join("themes"))?;
        let mut builder = ss.clone().into_builder();
        builder.add_from_folder(from.join("syntaxes"), true)?;
        ss = builder.build();

        syntect::dumps::dump_to_file(&themes, to.join("themes.bin"))?;
        syntect::dumps::dump_to_file(&ss, to.join("syntaxes.bin"))?;
        tracing::debug!("Cache build success");
        return Ok(());
    }

    let content = config
        .content
        .clone()
        .wrap_err("Cannot get content from args")?;
    let syntax = if let Some(ext) = &config.extension {
        ss.find_syntax_by_extension(ext)
            .wrap_err("Extension not found from extension argument")?
    } else {
        ss.find_syntax_for_file(content.filename())?
            .wrap_err("Extension not found from stdin or file")?
    };

    let theme = if let Some(vim_theme) = &config.vim_theme {
        Cow::Owned(theme_from_vim(vim_theme)?)
    } else {
        let theme = config
            .theme
            .clone()
            .unwrap_or("base16-ocean.dark".to_string());
        tracing::trace!("Trying load {theme:?}");
        themes
            .themes
            .get(&theme)
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(load_theme(&theme, false).unwrap()))
    };

    if theme.settings.background.is_some()
        && g_config.colors.windows_background
            == sss_lib::Background::Solid(sss_lib::image::Rgba([0x42, 0x87, 0xf5, 255]))
    {
        g_config.colors.windows_background = theme
            .settings
            .background
            .map(|c| sss_lib::Background::Solid(sss_lib::image::Rgba([c.r, c.g, c.b, c.a])))
            .ok_or(ConfigurationError::ParamNotFound("background".to_owned()))?
    }

    Ok(generate_image(
        g_config.clone(),
        ImageCode {
            config,
            syntax,
            theme,
            lib_config: g_config.clone(),
            syntax_set: &ss,
            content: &content.contents().expect("Cannot get content to render"),
            font: g_config.fonts,
        },
    )?)
}

fn list_file_types(ss: &SyntaxSet) {
    for s in ss.syntaxes() {
        println!("- {} (.{})", s.name, s.file_extensions.join(", ."));
    }
}
