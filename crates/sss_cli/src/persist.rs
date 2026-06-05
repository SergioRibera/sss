//! Persisted UI state — currently just the last interactive area
//! selection, so `--area` (without value) can re-open on the same rect.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sss_capture::Rect;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct StoredRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl From<Rect> for StoredRect {
    fn from(r: Rect) -> Self {
        Self {
            x: r.x(),
            y: r.y(),
            width: r.width(),
            height: r.height(),
        }
    }
}

impl From<StoredRect> for Rect {
    fn from(s: StoredRect) -> Self {
        Rect::from_xywh(s.x, s.y, s.width, s.height)
    }
}

fn file_path() -> Option<PathBuf> {
    let dir = directories::BaseDirs::new()?.config_dir().join("sss");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("last_selection.toml"))
}

pub fn load_last_area() -> Option<Rect> {
    let path = file_path()?;
    let body = std::fs::read_to_string(&path).ok()?;
    let stored: StoredRect = toml::from_str(&body).ok()?;
    Some(stored.into())
}

pub fn save_last_area(rect: Rect) {
    let Some(path) = file_path() else {
        return;
    };
    let stored = StoredRect::from(rect);
    match toml::to_string(&stored) {
        Ok(body) => {
            if let Err(e) = std::fs::write(&path, body) {
                tracing::warn!(error = %e, path = %path.display(), "could not persist last selection");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not serialise last selection");
        }
    }
}
