use sss_lib::DynImageContent;

use crate::config::CliConfig;
use crate::shot::ShotImpl;

pub struct Screenshot {
    pub config: CliConfig,
}

impl Screenshot {}

impl DynImageContent for Screenshot {
    fn content(&self) -> sss_lib::image::DynamicImage {
        let shot = ShotImpl::default();
        let img = if self.config.screen && self.config.current {
            screenshots::Screen::from_point(0, 0) // replace by mouse
                .unwrap()
                .capture()
                .unwrap()
        } else if let Some(area) = self.config.area {
            shot.capture_area(area, self.config.show_cursor).unwrap()
        } else {
            shot.all(self.config.show_cursor).unwrap()
        };

        sss_lib::image::DynamicImage::ImageRgba8(img)
    }
}
