use sss_lib::image::DynamicImage;
use sss_lib::DynImageContent;

use crate::config::CodeConfig;

pub struct ImageCode {
    pub config: CodeConfig,
}

impl DynImageContent for ImageCode {
    fn content(&self) -> DynamicImage {
        let mut img = DynamicImage::ImageRgb8(RgbImage::new(500, 500));

        let mut h = HighlightLines::new(syntax, &theme);
        for line in LinesWithEndings::from(&content) {
            let ranges: Vec<(Style, &str)> = h.highlight_line(line, &ss).unwrap();
            // ranges.iter().for_each(|(style, content)| {
            //     println!("Style: {style:?}\nContent: '{content}'");
            // });
        }

        img
    }
}
