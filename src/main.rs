use cli::get_args;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::{as_24_bit_terminal_escaped, LinesWithEndings};
use theme::load_themes;

mod cli;
mod error;
mod highlight;
mod theme;

fn main() {
    let args = get_args();
    load_themes();
    // Load these once at the start of your program
    let ps = SyntaxSet::load_defaults_newlines();

    // let themes = ThemeSet::default();
    //
    // let syntax = ps.find_syntax_by_extension("rs").unwrap();
    // let mut h = HighlightLines::new(syntax, &theme);
    // let s = "pub struct Wow { hi: u64 }\nfn blah() -> u64 {}";
    // for line in LinesWithEndings::from(s) {
    //     let ranges: Vec<(Style, &str)> = h.highlight_line(line, &ps).unwrap();
    //     ranges.iter().for_each(|(style, content)| {
    //         println!("Style: {style:?}\nContent: '{content}'");
    //         // let escaped = as_24_bit_terminal_escaped(&ranges[..], true);
    //         // print!("{}", escaped);
    //     });
    // }
}
