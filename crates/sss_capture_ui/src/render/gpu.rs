//! GPU rendering glue for the interactive overlay.

use std::sync::Arc;

use winit::window::Window;

/// Shared GPU state, one instance per app.
pub(crate) struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    pub fn new_instance() -> wgpu::Instance {
        wgpu::Instance::new(&wgpu::InstanceDescriptor::default())
    }

    /// `compatible_surface` must outlive the returned `Gpu`; wgpu keeps an
    /// internal reference to it until the adapter is dropped.
    pub fn new_with_surface(
        instance: wgpu::Instance,
        compatible_surface: &wgpu::Surface<'_>,
    ) -> Result<Self, String> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(compatible_surface),
            force_fallback_adapter: false,
        }))
        .map_err(|e| e.to_string())?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("sss_capture_ui device"),
            required_features: wgpu::Features::empty(),
            required_limits:
                wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("wgpu device: {e}"))?;

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }
}

/// Per-overlay-window GPU + egui state.
pub(crate) struct WindowGpu {
    pub surface: wgpu::Surface<'static>,
    pub surface_format: wgpu::TextureFormat,
    pub config: wgpu::SurfaceConfiguration,
    pub egui_ctx: egui::Context,
    pub egui_winit: egui_winit::State,
    pub renderer: egui_wgpu::Renderer,
    pub size: (u32, u32),
}

impl WindowGpu {
    /// Build for the surface that was used to create the adapter.
    pub fn from_surface(
        window: Arc<dyn Window>,
        surface: wgpu::Surface<'static>,
        gpu: &Gpu,
    ) -> Result<Self, String> {
        Self::finish(window, surface, gpu)
    }

    /// Build for a window that doesn't have a surface yet.
    pub fn new(window: Arc<dyn Window>, gpu: &Gpu) -> Result<Self, String> {
        let surface = gpu
            .instance
            .create_surface(window.clone())
            .map_err(|e| format!("create_surface: {e}"))?;
        Self::finish(window, surface, gpu)
    }

    fn finish(
        window: Arc<dyn Window>,
        surface: wgpu::Surface<'static>,
        gpu: &Gpu,
    ) -> Result<Self, String> {
        let size = {
            let s = window.surface_size();
            (s.width.max(1), s.height.max(1))
        };

        let caps = surface.get_capabilities(&gpu.adapter);
        let surface_format = caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        // Fifo + frame-latency 1: redraw is event-driven, and on multi-monitor
        // setups deeper queues have hard-locked the kernel via the GPU driver.
        // Opaque alpha lets Wayland compositors skip per-pixel blending.
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.0,
            height: size.1,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode: caps
                .alpha_modes
                .iter()
                .copied()
                .find(|m| matches!(m, wgpu::CompositeAlphaMode::Opaque))
                .or_else(|| caps.alpha_modes.first().copied())
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: Vec::new(),
        };
        surface.configure(&gpu.device, &config);

        let egui_ctx = egui::Context::default();
        let viewport_id = egui_ctx.viewport_id();
        let egui_winit = egui_winit::State::new(
            egui_ctx.clone(),
            viewport_id,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let renderer = egui_wgpu::Renderer::new(&gpu.device, surface_format, None, 1, false);

        Ok(Self {
            surface,
            surface_format,
            config,
            egui_ctx,
            egui_winit,
            renderer,
            size,
        })
    }

    pub fn resize(&mut self, gpu: &Gpu, new_size: (u32, u32)) {
        if new_size.0 == 0 || new_size.1 == 0 || new_size == self.size {
            return;
        }
        self.size = new_size;
        self.config.width = new_size.0;
        self.config.height = new_size.1;
        self.surface.configure(&gpu.device, &self.config);
    }
}
