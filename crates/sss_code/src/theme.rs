use std::fs::{self, read_dir};
use std::path::PathBuf;

use lewp_css::domain::{CssRuleType, StyleRule};
use lewp_css::Stylesheet;
use syntect::highlighting::{Theme, ThemeSet, ThemeSettings};

fn load_from_css(file: &PathBuf) -> (String, Theme) {
    let file_name = file.file_name().unwrap().to_str().unwrap().to_string();
    let file_name = file_name.split(".").next().unwrap().to_string();
    let content = fs::read_to_string(file).unwrap();
    let style = Stylesheet::parse(&content).unwrap();
    let style = style.rules.0.iter();
    let theme = Theme {
        name: Some(file_name.clone()),
        author: None,
        settings: ThemeSettings {
            foreground: style.find_map(|s| {
                if let CssRuleType::Style(StyleRule {selectors, property_declarations}) = s.rule_type() {
                } else {
                    None
                }
            }),
            background: todo!(),
            caret: todo!(),
            line_highlight: todo!(),
            misspelling: todo!(),
            minimap_border: todo!(),
            accent: todo!(),
            popup_css: todo!(),
            phantom_css: todo!(),
            bracket_contents_foreground: todo!(),
            bracket_contents_options: todo!(),
            brackets_foreground: todo!(),
            brackets_background: todo!(),
            brackets_options: todo!(),
            tags_foreground: todo!(),
            tags_options: todo!(),
            highlight: todo!(),
            find_highlight: todo!(),
            find_highlight_foreground: todo!(),
            gutter: todo!(),
            gutter_foreground: todo!(),
            selection: todo!(),
            selection_foreground: todo!(),
            selection_border: todo!(),
            inactive_selection: todo!(),
            inactive_selection_foreground: todo!(),
            guide: todo!(),
            active_guide: todo!(),
            stack_guide: todo!(),
            shadow: todo!(),
        },
        scopes: todo!(),
    };

    (file_name, theme)
}

pub fn load_themes() {
    let mut themes = ThemeSet::load_defaults();
    // themes.add_from_folder("./assets").unwrap_or_default();

    let dir_content = read_dir("./assets").unwrap();

    for file in dir_content {
        let Ok(file) = file else {
            continue;
        };
        let (theme_name, theme) = load_from_css(&file.path());
        themes.themes.insert(theme_name, theme);
    }

    syntect::dumps::dump_to_file(&themes, "./assets/themes.themedump").unwrap();
}
