use config::Position;
use display_info::DisplayInfo;
use iced::multi_window::Application;
use iced::{Point, Settings};

use app::MainApp;
pub use config::Config;

mod app;
mod config;

const MARGIN: f32 = 20.;
const PADDING: f32 = 8.;
const SIZE: f32 = 50.;

fn position_to_iced(pos: &Position, screen: &DisplayInfo, (width, height): (f32, f32)) -> Point {
    let DisplayInfo {
        x,
        y,
        width: w,
        height: h,
        ..
    } = screen;
    let (x, y) = (*x as f32, *y as f32);
    let (w, h) = (*w as f32, *h as f32);
    let (cx, cy) = (x + (w / 2.) - (width / 2.), y + (h / 2.) - (height / 2.));
    match pos {
        Position::Left => Point::new(x + MARGIN, cy),
        Position::Right => Point::new(x + w - width - MARGIN, cy),
        Position::Top => Point::new(cx, y + MARGIN),
        Position::Bottom => Point::new(cx, y + h - MARGIN - height),
    }
}

fn main() -> iced::Result {
    let settings = config::get_config();
    let size = settings.get_size();
    let screens = DisplayInfo::all().unwrap();
    let mut screens = screens.iter().map(|s| iced::window::Settings {
        position: settings
            .position
            .as_ref()
            .map(|pos| iced::window::Position::Specific(position_to_iced(pos, s, size)))
            .unwrap_or(iced::window::Position::Centered),
        size: iced::Size::new(size.0, size.1),
        visible: true,
        resizable: false,
        decorations: false,
        level: iced::window::Level::AlwaysOnTop,
        ..Default::default()
    });
    let window = screens.next().unwrap();

    MainApp::run(Settings {
        window,
        flags: (screens.collect::<Vec<iced::window::Settings>>(), settings),
        ..Default::default()
    })
}
