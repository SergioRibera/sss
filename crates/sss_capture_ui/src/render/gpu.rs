//! GPU rendering glue for the interactive overlay.
//!
//! Per-window wgpu surface + egui-wgpu renderer + egui-winit state. Only
//! built when the `editor` feature is on; the no-editor build path stays
//! fully CPU and renders nothing through this module.

use std::sync::Arc;

use winit::window::Window;

/// Shared GPU state — one instance per app, not per window.
pub(crate) struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// Build a wgpu instance.
    pub fn new_instance() -> wgpu::Instance {
        wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        })
    }

    /// Request an adapter + device using `compatible_surface` for adapter
    /// selection.
    ///
    /// Important: wgpu 22 keeps an internal reference to the surface used
    /// here until the adapter is dropped. **Do not drop the surface
    /// passed in here before the returned `Gpu` is finished with the
    /// adapter.** The caller normally moves the surface into a
    /// [`WindowGpu`] right after so it stays alive.
    pub fn new_with_surface(
        instance: wgpu::Instance,
        compatible_surface: &wgpu::Surface<'_>,
    ) -> Result<Self, String> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(compatible_surface),
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| "wgpu: no adapter".to_string())?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("sss_capture_ui device"),
                required_features: wgpu::Features::empty(),
                required_limits:
                    wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        ))
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
    /// Build a [`WindowGpu`] for an *already-created* surface. Use this when
    /// the surface had to exist before the adapter (i.e. the first window
    /// whose surface fed `Gpu::new_with_surface`).
    pub fn from_surface(
        window: Arc<Window>,
        surface: wgpu::Surface<'static>,
        gpu: &Gpu,
    ) -> Result<Self, String> {
        Self::finish(window, surface, gpu)
    }

    /// Build a [`WindowGpu`] for a window that doesn't have a surface yet.
    /// The adapter must already exist (this is the path for windows 2..N).
    pub fn new(window: Arc<Window>, gpu: &Gpu) -> Result<Self, String> {
        let surface = gpu
            .instance
            .create_surface(window.clone())
            .map_err(|e| format!("create_surface: {e}"))?;
        Self::finish(window, surface, gpu)
    }

    fn finish(
        window: Arc<Window>,
        surface: wgpu::Surface<'static>,
        gpu: &Gpu,
    ) -> Result<Self, String> {
        let size = {
            let s = window.inner_size();
            (s.width.max(1), s.height.max(1))
        };

        let caps = surface.get_capabilities(&gpu.adapter);
        let surface_format = caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        // Always Fifo: an annotation overlay is an editor, not a game. We
        // redraw on input events only, so unbounded mailbox-style framerates
        // are pure waste and — on multi-monitor setups (4 surfaces simul-
        // taneously) — were enough to hard-lock the kernel via the GPU
        // driver. Pin `desired_maximum_frame_latency` to 1 for the same
        // reason: minimum queue depth, no head-of-line build-up.
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.0,
            height: size.1,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            // Opaque alpha mode forces wgpu to declare the surface opaque
            // to the compositor (when supported). Wayland compositors then
            // skip per-pixel blending for the overlay, which avoids a
            // second source of multi-surface compositing pressure.
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
            &*window,
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
