use std::collections::HashMap;

use iced::widget::{svg, text, tooltip, Button, Column, Row};
use iced::window::Id;
use iced::{multi_window, Command, Length};

use crate::config::Position;
use crate::{Config, PADDING, SIZE};

pub struct MainApp {
    windows: HashMap<Id, Window>,
    config: Config,
    icon_all: svg::Handle,
    icon_screen: svg::Handle,
    icon_area: svg::Handle,
}

#[derive(Debug)]
struct Window;

#[derive(Debug, Clone)]
pub enum MainMessage {
    All,
    Screen,
    Area,
}

impl ToString for MainMessage {
    fn to_string(&self) -> String {
        match self {
            MainMessage::All => "Capture All Screens".to_string(),
            MainMessage::Screen => "Capture Curren Screen".to_string(),
            MainMessage::Area => "Capture Area".to_string(),
        }
    }
}

impl multi_window::Application for MainApp {
    type Executor = iced::executor::Default;
    type Message = MainMessage;
    type Theme = iced::Theme;
    type Flags = (Vec<iced::window::Settings>, Config);

    fn new((raw_windows, config): Self::Flags) -> (Self, Command<Self::Message>) {
        let mut windows = HashMap::new();
        let commands = raw_windows
            .iter()
            .map(|w| {
                let (id, c) = iced::window::spawn(w.clone());
                windows.insert(id, Window);
                c
            })
            .collect::<Vec<Command<MainMessage>>>();
        (
            Self {
                windows,
                config,
                icon_all: svg::Handle::from_path(format!(
                    "{}/crates/launcher/assets/all.svg",
                    env!("CARGO_MANIFEST_DIR")
                )),
                icon_screen: svg::Handle::from_path(format!(
                    "{}/crates/launcher/assets/screen.svg",
                    env!("CARGO_MANIFEST_DIR")
                )),
                icon_area: svg::Handle::from_path(format!(
                    "{}/crates/launcher/assets/area.svg",
                    env!("CARGO_MANIFEST_DIR")
                )),
            },
            Command::batch(commands),
        )
    }

    fn title(&self, _window: iced::window::Id) -> String {
        "SSS Launcher".to_string()
    }

    fn update(&mut self, message: Self::Message) -> iced::Command<Self::Message> {
        let _cmd = match message {
            MainMessage::All => self.config.all_command.as_deref().unwrap_or_default(),
            MainMessage::Screen => self.config.screen_command.as_deref().unwrap_or_default(),
            MainMessage::Area => self.config.area_command.as_deref().unwrap_or_default(),
        };
        Command::none()
    }

    fn view(
        &self,
        id: iced::window::Id,
    ) -> iced::Element<'_, Self::Message, Self::Theme, iced::Renderer> {
        self.windows.get(&id).unwrap().view(
            self.config.position.clone().unwrap_or_default(),
            self.config.area_command.is_some(),
            self.config.all_command.is_some(),
            self.config.screen_command.is_some(),
            self.icon_area.clone(),
            self.icon_all.clone(),
            self.icon_screen.clone(),
        )
    }
}

impl Window {
    fn view(
        &self,
        position: crate::Position,
        area_command: bool,
        all_command: bool,
        screen_command: bool,
        icon_area: svg::Handle,
        icon_all: svg::Handle,
        icon_screen: svg::Handle,
    ) -> iced::Element<'_, MainMessage, iced::Theme, iced::Renderer> {
        match position {
            Position::Top | Position::Bottom => horizontal(
                area_command,
                all_command,
                screen_command,
                icon_area,
                icon_all,
                icon_screen,
            )
            .into(),
            _ => vertical(
                area_command,
                all_command,
                screen_command,
                icon_area,
                icon_all,
                icon_screen,
            )
            .into(),
        }
    }
}

fn btn(
    icon: svg::Handle,
    msg: MainMessage,
) -> impl Into<iced::Element<'static, MainMessage, iced::Theme, iced::Renderer>> {
    tooltip(
        Button::new(
            svg(icon)
                .width(Length::Fixed(SIZE))
                .height(Length::Fixed(SIZE)),
        )
        .on_press(msg.clone()),
        text(msg.to_string()),
        tooltip::Position::FollowCursor,
    )
}

fn horizontal(
    area_command: bool,
    all_command: bool,
    screen_command: bool,
    icon_area: svg::Handle,
    icon_all: svg::Handle,
    icon_screen: svg::Handle,
) -> Row<'static, MainMessage> {
    Row::new()
        .padding(PADDING)
        .spacing(PADDING)
        .push_maybe(area_command.then_some(btn(icon_area, MainMessage::Area)))
        .push_maybe(all_command.then_some(btn(icon_all, MainMessage::All)))
        .push_maybe(screen_command.then_some(btn(icon_screen, MainMessage::Screen)))
}

fn vertical(
    area_command: bool,
    all_command: bool,
    screen_command: bool,
    icon_area: svg::Handle,
    icon_all: svg::Handle,
    icon_screen: svg::Handle,
) -> Column<'static, MainMessage> {
    Column::new()
        .padding(PADDING)
        .spacing(PADDING)
        .push_maybe(area_command.then_some(btn(icon_area, MainMessage::Area)))
        .push_maybe(all_command.then_some(btn(icon_all, MainMessage::All)))
        .push_maybe(screen_command.then_some(btn(icon_screen, MainMessage::Screen)))
}
