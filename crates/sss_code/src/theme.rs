use std::path::Path;

use syntect::dumps::{dump_to_file, from_dump_file};
use syntect::highlighting::{Theme, ThemeSet};

mod color;
mod parser;
mod vim;

pub use vim::theme_from_vim;

pub fn list_themes(ts: &ThemeSet) {
    for t in ts.themes.keys() {
        println!("- {}", t);
    }
}

pub fn load_theme(tm_file: &str, enable_caching: bool) -> Theme {
    let tm_path = Path::new(tm_file);

    if enable_caching {
        let tm_cache = tm_path.with_extension("tmdump");

        if tm_cache.exists() {
            from_dump_file(tm_cache).unwrap()
        } else {
            let theme = ThemeSet::get_theme(tm_path).unwrap();
            dump_to_file(&theme, tm_cache).unwrap();
            theme
        }
    } else {
        ThemeSet::get_theme(tm_path).unwrap()
    }
}
