use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use winit::window::Window;

/// Everything wgpu needs to draw into the window. The engine shares `device`
/// and `queue` (see `GpuContext::from_device`); nothing else creates a device.
pub struct Gfx {
    pub instance: wgpu::Instance,
    pub surface: wgpu::Surface<'static>,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub info: wgpu::AdapterInfo,
}

impl Gfx {
    /// Another OS surface on the existing GPU. Dialogs never allocate their
    /// own device or compete with the render engine for an adapter.
    pub fn sibling(&self, window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();
        let surface = self.instance.create_surface(window)?;
        let caps = surface.get_capabilities(&self.adapter);
        anyhow::ensure!(
            caps.formats.contains(&self.config.format),
            "dialog surface format unavailable"
        );
        let mut config = self.config.clone();
        config.width = size.width.max(1);
        config.height = size.height.max(1);
        surface.configure(&self.device, &config);
        Ok(Self {
            instance: self.instance.clone(),
            surface,
            adapter: self.adapter.clone(),
            device: self.device.clone(),
            queue: self.queue.clone(),
            config,
            info: self.info.clone(),
        })
    }

    pub fn new(window: Arc<Window>) -> Result<Self> {
        pollster::block_on(Self::new_async(window))
    }

    async fn new_async(window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12,
            ..wgpu::InstanceDescriptor::new_with_display_handle(Box::new(window.clone()))
        });
        let surface = instance
            .create_surface(window.clone())
            .context("create_surface")?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .context("request_adapter")?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("syfrack-studio"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            })
            .await
            .context("request_device")?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .ok_or_else(|| anyhow!("no sRGB surface format among {:?}", caps.formats))?;
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| anyhow!("surface not supported by adapter"))?;
        config.format = format;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);
        Ok(Self {
            instance,
            surface,
            adapter,
            device,
            queue,
            config,
            info,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Next swapchain texture, or `None` when there is nothing to draw this frame.
    pub fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        if self.config.width == 0 || self.config.height == 0 {
            return None;
        }
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(t),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => None,
            _ => {
                // Outdated or lost: reconfigure and skip this frame.
                self.surface.configure(&self.device, &self.config);
                None
            }
        }
    }
}
