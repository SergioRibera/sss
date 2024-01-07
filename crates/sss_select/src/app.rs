use std::num::NonZeroU32;
use std::rc::Rc;

use raqote::{DrawTarget, SolidSource};
use softbuffer::{Context, Surface};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::window::Window;

use crate::{Monitor, SelectConfig};

const SCREEN_RECT_WIDTH: f32 = 200.;
const SCREEN_RECT_HEIGHT: f32 = 150.;

pub struct SelectApp {
    config: SelectConfig,
    dt: DrawTarget,
    dragging: bool,
    monitors: Vec<Monitor>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    background: SolidSource,
    background_opaque: SolidSource,
    foreground: SolidSource,
    foreground_intensive: SolidSource,
    size: PhysicalSize<u32>,
    start_pos: PhysicalPosition<f64>,
    end_pos: PhysicalPosition<f64>,
    mouse_pos: PhysicalPosition<f64>,
}

impl SelectApp {
    pub fn new(window: Rc<Window>, monitors: Vec<Monitor>, config: SelectConfig) -> Self {
        let context = Context::new(window.clone()).unwrap();
        let size = window.inner_size();

        Self {
            size,
            config,
            monitors,
            dragging: false,
            dt: DrawTarget::new(size.width as i32, size.height as i32),
            surface: Surface::new(&context, window.clone()).unwrap(),
            start_pos: PhysicalPosition::default(),
            end_pos: PhysicalPosition::default(),
            mouse_pos: PhysicalPosition::default(),
            background: SolidSource {
                r: 255,
                g: 255,
                b: 255,
                a: 127,
            },
            background_opaque: SolidSource {
                r: 0,
                g: 0,
                b: 0,
                a: 150,
            },
            foreground: SolidSource {
                r: 255,
                g: 255,
                b: 255,
                a: 180,
            },
            foreground_intensive: SolidSource {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
        }
    }

    pub fn render_region(&mut self) {}
    pub fn render_screen(&mut self) {
        for m in &self.monitors {
            let PhysicalPosition { x, y } = m.position;
            let PhysicalSize { width, height } = m.size;
            self.dt.fill_rect(
                (x + width as i32) as f32 - SCREEN_RECT_WIDTH - 20.,
                (y + height as i32) as f32 - SCREEN_RECT_HEIGHT - 20.,
                SCREEN_RECT_WIDTH,
                SCREEN_RECT_HEIGHT,
                &raqote::Source::Solid(self.background_opaque.clone()),
                &raqote::DrawOptions::default(),
            )
        }
    }

    pub fn update_mouse(&mut self, mouse_pos: Option<PhysicalPosition<f64>>, start: Option<bool>) {
        if let Some(pos) = mouse_pos {
            self.mouse_pos = pos;
        }
        if let Some(s) = start {
            self.dragging = s;
            if s {
                self.start_pos = self.mouse_pos;
            } else {
                self.end_pos = self.mouse_pos;
            }
        }
    }

    pub fn pre_render(&mut self) {
        self.surface
            .resize(
                NonZeroU32::new(self.size.width).unwrap(),
                NonZeroU32::new(self.size.height).unwrap(),
            )
            .unwrap();
        self.dt.clear(self.background.clone());
    }

    pub fn render(&mut self) {
        let mut buffer = self.surface.buffer_mut().unwrap();

        self.dt
            .get_data()
            .iter()
            .enumerate()
            .for_each(|(i, px)| buffer[i] = *px);

        buffer.present().unwrap();
    }
}
