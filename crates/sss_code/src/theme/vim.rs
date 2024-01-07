use std::str::FromStr;

use syntect::highlighting::{
    FontStyle, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings, UnderlineOption,
};

use super::parser::{parse_vim, vim_to_scope_str};

fn font_style(v: String) -> Option<FontStyle> {
    match v.to_lowercase().as_str() {
        "bold" => Some(FontStyle::BOLD),
        "italic" => Some(FontStyle::ITALIC),
        "underline" => Some(FontStyle::UNDERLINE),
        _ => None,
    }
}

fn underline(v: String) -> UnderlineOption {
    match v.to_lowercase().as_str() {
        "underline" => UnderlineOption::Underline,
        "undercurl" => UnderlineOption::SquigglyUnderline,
        "underdotted" => UnderlineOption::StippledUnderline,
        _ => UnderlineOption::None,
    }
}

pub fn theme_from_vim(vim: &str) -> Theme {
    let values = parse_vim(vim);
    let scopes = values
        .iter()
        .map(|(name, (fg, bg, style))| ThemeItem {
            scope: vim_to_scope_str(name)
                .and_then(|v| ScopeSelectors::from_str(v).ok())
                .unwrap_or_default(),
            style: StyleModifier {
                foreground: *fg,
                background: *bg,
                font_style: style.clone().and_then(font_style),
            },
        })
        .collect::<Vec<_>>();

    let &(fg_n, bg_n, _) = values.get("Normal").unwrap();
    let &(fg_nr, bg_nr, _) = values.get("LineNr").unwrap();
    let &(fg_sel, bg_sel, _) = values.get("Visual").unwrap();
    let &(_, bg_cur, _) = values.get("Cursor").unwrap();
    let &(_, bg_cur_line, _) = values.get("CursorLine").unwrap();
    let &(fg_find, bg_find, _) = values.get("Search").unwrap();
    let &(fg_bad, _, _) = values.get("SpellBad").unwrap();
    let &(fg_tag, _, _) = values.get("Title").unwrap();
    let (fg_brk, bg_brk, s_brk) = values.get("MatchParen").unwrap();
    let &(fg_ibl, _, _) = values.get("IdentBlanklineChar").unwrap();

    Theme {
        scopes,
        settings: ThemeSettings {
            foreground: fg_n,
            background: bg_n,
            caret: bg_cur,
            line_highlight: bg_cur_line,
            misspelling: fg_bad,
            accent: fg_n,
            bracket_contents_foreground: *fg_brk,
            bracket_contents_options: Some(underline(s_brk.clone().unwrap_or_default())),
            brackets_foreground: *fg_brk,
            brackets_background: *bg_brk,
            brackets_options: None,
            tags_foreground: fg_tag,
            tags_options: None,
            highlight: None,
            find_highlight: fg_find,
            find_highlight_foreground: bg_find,
            gutter: fg_nr,
            gutter_foreground: bg_nr,
            selection: bg_sel,
            selection_foreground: fg_sel,
            selection_border: bg_sel,
            inactive_selection: bg_n,
            inactive_selection_foreground: fg_n,
            guide: None,
            active_guide: fg_ibl,
            stack_guide: fg_ibl,
            ..Default::default()
        },
        ..Default::default()
    }
}
