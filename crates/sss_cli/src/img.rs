use mouse_position::{Mouse, MouseExt};
use sss_lib::error::ImagenGeneration as ImagenGenerationError;
use sss_lib::image::RgbaImage;
use sss_lib::DynImageContent;

use crate::config::CliConfig;
use crate::shot::ShotImpl;

pub struct Screenshot {
    pub config: CliConfig,
}

impl Screenshot {}

impl DynImageContent for Screenshot {
    fn content(&self) -> Result<RgbaImage, ImagenGenerationError> {
        tracing::trace!("Generating Image");
        let shot = ShotImpl::new().map_err(|e| ImagenGenerationError::Custom(e.to_string()))?;

        if self.config.screen && self.config.current {
            tracing::trace!("Capture current screen");
            let (x, y) = Mouse::default().get_pos().map_err(|e| {
                ImagenGenerationError::Custom(format!("Cannot get mouse position: {e:?}"))
            })?;

            shot.screen(Some((x, y)), None, None, self.config.show_cursor)
        } else if let Some(area) = self.config.area {
            tracing::trace!("Capture area");
            shot.capture_area(area, self.config.show_cursor)
        } else if let Some(id) = self.config.screen_id.as_ref() {
            let name = Some(id.clone());
            let id = id.parse::<i32>().ok();
            tracing::trace!("Capture specific screen: {{ name: {name:?}, id: {id:?} }}");
            shot.screen(None, id, name, self.config.show_cursor)
        } else {
            tracing::trace!("Capture all screens");
            shot.all(self.config.show_cursor)
        }
    }
}
