use mouse_position::{Mouse, MouseExt};
use sss_lib::error::ImagenGeneration as ImagenGenerationError;
use sss_lib::image::RgbaImage;
use sss_lib::DynImageContent;

use crate::config::DirectTarget;
use crate::shot::ShotImpl;

/// Two ways to build a screenshot: drive the platform via [`ShotImpl`] using
/// an already-resolved [`DirectTarget`], or hand back an image produced by
/// the interactive selector (already decorated with the user's annotations).
pub enum Screenshot {
    Direct {
        target: DirectTarget,
        show_cursor: bool,
    },
    PreRendered(RgbaImage),
}

impl Screenshot {
    pub fn from_target(target: DirectTarget, show_cursor: bool) -> Self {
        Self::Direct {
            target,
            show_cursor,
        }
    }
    pub fn pre_rendered(image: RgbaImage) -> Self {
        Self::PreRendered(image)
    }
}

impl DynImageContent for Screenshot {
    fn content(&self) -> Result<RgbaImage, ImagenGenerationError> {
        match self {
            Screenshot::PreRendered(img) => Ok(img.clone()),
            Screenshot::Direct {
                target,
                show_cursor,
            } => {
                tracing::trace!("Generating Image: {target:?}");
                let shot = ShotImpl::new(*show_cursor)
                    .map_err(|e| ImagenGenerationError::Custom(e.to_string()))?;

                match target {
                    DirectTarget::CurrentMonitor => {
                        let (x, y) = Mouse::default().get_pos().map_err(|e| {
                            ImagenGenerationError::Custom(format!(
                                "Cannot get mouse position: {e:?}"
                            ))
                        })?;
                        shot.screen(Some((x, y)), None, None)
                    }
                    DirectTarget::Area(area) => shot.capture_area(*area),
                    DirectTarget::Screen(value) => {
                        let id = value.parse::<i32>().ok();
                        shot.screen(None, id, Some(value.clone()))
                    }
                    DirectTarget::Window(value) => shot.window(value),
                }
            }
        }
    }
}
