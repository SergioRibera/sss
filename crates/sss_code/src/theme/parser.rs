use std::collections::HashMap;

use syntect::highlighting::Color;

use super::color::str_to_color;

pub type VimHighlight = (Option<Color>, Option<Color>, Option<String>);

pub fn parse_vim(vim: &str) -> HashMap<&str, VimHighlight> {
    vim.split(";")
        .map(|group| {
            let mut values = group.splitn(4, ",");
            let name = values.next().unwrap();
            let bg = values.next().and_then(str_to_color);
            let fg = values.next().and_then(str_to_color);
            let style = values
                .next()
                .and_then(|v| (!v.is_empty()).then_some(v.to_string()));
            (name, (bg, fg, style))
        })
        .collect::<HashMap<&str, VimHighlight>>()
}

const VIM_NAMES: [(&str, &str); 12] = [
    ("Number", "constant.numeric"),
    ("Character", "constant.character"),
    ("String", "string"),
    ("Constant", "constant"),
    ("Identifier", "variable"),
    ("Keyword", "keyword"),
    ("Comment", "comment"),
    ("Operator", "keyword.operator"),
    ("Statement", "variable.parameter.function"),
    ("Type", "entity.name.class, meta.class, support.class"),
    ("StorageClass", "storage"),
    ("Function", "entity.name.function, support.function"),
];

pub fn vim_to_scope_str(v: &str) -> Option<&str> {
    VIM_NAMES.iter().find(|(n, _)| n == &v).map(|(_, v)| *v)
}
