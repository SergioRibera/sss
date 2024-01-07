
use std::rc::Rc;

use app::SelectApp;

use utils::calculate_layout_size;
use winit::dpi::{LogicalPosition, PhysicalPosition, PhysicalSize};
use winit::event::MouseButton;

use winit::{
    event::{Event, WindowEvent},
    event_loop::EventLoop,
    window::WindowBuilder,
};

mod app;
mod utils;

#[derive(Clone, Default)]
pub struct Monitor {
    pub name: Option<String>,
    pub size: PhysicalSize<u32>,
    pub position: PhysicalPosition<i32>,
}

#[derive(Clone, Default)]
pub struct Area {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Clone, Debug, Default)]
pub struct SelectConfig {
    /// When you take from a screen or window,
    /// capture the one on which the mouse is located.
    pub current: bool,
    /// Capture a full screen
    pub screen: bool,
    /// Capture a application window
    pub window: bool,
    /// Captures an area of the screen
    pub area: bool,
}

pub fn get_area(config: SelectConfig) -> Area {
    let area = Area::default();
    let event_loop = EventLoop::new().unwrap();

    let window = Rc::new(
        WindowBuilder::new()
            .with_title("sss")
            .with_visible(true)
            .with_resizable(false)
            .with_transparent(true)
            .with_decorations(false)
            .with_maximized(true)
            .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
            .build(&event_loop)
            .unwrap(),
    );

    let monitors = window
        .available_monitors()
        .map(|m| Monitor {
            name: m.name(),
            size: m.size(),
            position: m.position(),
        })
        .collect::<Vec<_>>();

    let new_size = calculate_layout_size(&monitors);
    println!("New Size: {new_size:?}");
    window.set_outer_position(LogicalPosition::new(0, 0));
    window.set_min_inner_size(Some(new_size));
    window.set_max_inner_size(Some(new_size));

    if config.area || config.screen && !config.current {
        let mut app = SelectApp::new(window.clone(), monitors, config.clone());

        event_loop
            .run(move |event, elwt| {
                elwt.set_control_flow(winit::event_loop::ControlFlow::Wait);
                println!("{event:?}");

                if let Event::WindowEvent { event, .. } = event {
                    match event {
                        WindowEvent::CloseRequested => elwt.exit(),
                        WindowEvent::CursorMoved { position, .. } => {
                            app.update_mouse(Some(position), None);
                            window.request_redraw();
                        }
                        WindowEvent::MouseInput { state, button, .. } => {
                            if button == MouseButton::Left && state.is_pressed() {
                                app.update_mouse(None, Some(true));
                            }
                            if button == MouseButton::Left && !state.is_pressed() {
                                app.update_mouse(None, Some(false));
                            }
                            window.request_redraw();
                        }
                        WindowEvent::RedrawRequested => {
                            app.pre_render();
                            if config.area {
                                app.render_region();
                            }

                            if config.screen {
                                app.render_screen();
                            }

                            app.render();
                        }
                        _ => (),
                    }
                }
            })
            .unwrap();
    }

    area
}
