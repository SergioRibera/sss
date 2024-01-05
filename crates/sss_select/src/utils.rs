use winit::dpi::{PhysicalPosition, PhysicalSize};

use crate::Monitor;

pub fn calculate_layout_size(monitors: &[Monitor]) -> PhysicalSize<i32> {
    let mut max_x = 0;
    let mut max_y = 0;

    for monitor in monitors {
        let PhysicalSize { width, height } = monitor.size;
        let PhysicalPosition { x, y } = monitor.position;

        // Sumamos el ancho y la altura del monitor a la posición máxima
        max_x = max_x.max(x + width as i32);
        max_y = max_y.max(y + height as i32);
    }

    PhysicalSize::new(max_x, max_y)
}
