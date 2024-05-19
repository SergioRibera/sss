use std::path::Path;

use syntect::dumps::{dump_to_file, from_dump_file};
use syntect::highlighting::{Theme, ThemeSet};

mod color;
mod parser;
mod vim;

pub use vim::theme_from_vim;

use crate::error::CodeScreenshot;

pub fn list_themes(ts: &ThemeSet) {
    for t in ts.themes.keys() {
        println!("- {}", t);
    }
}

pub fn load_theme(tm_file: &str, enable_caching: bool) -> Result<Theme, CodeScreenshot> {
    let tm_path = Path::new(tm_file);

    if enable_caching {
        tracing::info!("Finding theme in cache");
        let tm_cache = tm_path.with_extension("tmdump");

        if tm_cache.exists() {
            tracing::debug!("Loading theme {tm_path:?} from cache");
            Ok(from_dump_file(tm_cache).unwrap())
        } else {
            tracing::debug!("Loading theme {tm_path:?} from ThemeSet");
            let theme = ThemeSet::get_theme(tm_path)?;
            dump_to_file(&theme, tm_cache).unwrap();
            tracing::info!("Updating cache");
            Ok(theme)
        }
    } else {
        tracing::info!("Loading theme {tm_path:?}");
        Ok(ThemeSet::get_theme(tm_path)?)
    }
}
