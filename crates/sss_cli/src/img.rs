use mouse_position::mouse_position::Mouse;
use sss_lib::DynImageContent;

use crate::config::CliConfig;
use crate::shot::ShotImpl;

pub struct Screenshot {
    pub config: CliConfig,
}

impl Screenshot {}

impl DynImageContent for Screenshot {
    fn content(&self) -> sss_lib::image::RgbaImage {
        let shot = ShotImpl::default();

        if self.config.screen && self.config.current {
            let Mouse::Position { x, y } = Mouse::get_mouse_position() else {
                panic!("Cannot get mouse position");
            };
            screenshots::Screen::from_point(x, y) // replace by mouse
                .unwrap()
                .capture()
                .unwrap()
        } else if let Some(area) = self.config.area {
            shot.capture_area(area, self.config.show_cursor).unwrap()
        } else if let Some(id) = self.config.screen_id.as_ref() {
            let name = Some(id.clone());
            let id = id.parse::<i32>().ok();
            shot.screen(None, id, name, self.config.show_cursor)
                .unwrap()
        } else {
            shot.all(self.config.show_cursor).unwrap()
        }
    }
}
