use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};

/// A GPU device and queue, obtained by trying backends in the order
/// Vulkan, DX12, OpenGL (the order the spec mandates for the Kepler target).
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    errors: Arc<Mutex<Vec<String>>>,
}

impl GpuContext {
    pub fn new() -> Result<Self> {
        let mut errors = Vec::new();
        for backends in [
            wgpu::Backends::VULKAN,
            wgpu::Backends::DX12,
            wgpu::Backends::GL,
        ] {
            match Self::try_backend(backends) {
                Ok(ctx) => return Ok(ctx),
                Err(e) => errors.push(format!("{backends:?}: {e:#}")),
            }
        }
        Err(anyhow!("no usable GPU backend:\n{}", errors.join("\n")))
    }

    fn try_backend(backends: wgpu::Backends) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .context("request_adapter")?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("syfrack"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .context("request_device")?;
        Ok(Self::from_device(device, queue, info))
    }

    /// Wraps an existing device (for example the window's) and installs the
    /// error inbox callbacks. The engine never creates a second device when
    /// a window already owns one.
    pub fn from_device(device: wgpu::Device, queue: wgpu::Queue, info: wgpu::AdapterInfo) -> Self {
        let errors = Arc::new(Mutex::new(Vec::new()));
        {
            let sink = errors.clone();
            device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
                if let Ok(mut v) = sink.lock() {
                    v.push(format!("uncaptured: {e}"));
                }
            }));
            let sink = errors.clone();
            device.set_device_lost_callback(move |reason, msg| {
                if let Ok(mut v) = sink.lock() {
                    v.push(format!("device lost ({reason:?}): {msg}"));
                }
            });
        }
        Self {
            device,
            queue,
            info,
            errors,
        }
    }

    /// Drains the errors reported outside any error scope (uncaptured errors,
    /// device loss).
    pub fn take_errors(&self) -> Vec<String> {
        match self.errors.lock() {
            Ok(mut v) => std::mem::take(&mut *v),
            Err(p) => std::mem::take(&mut *p.into_inner()),
        }
    }

    /// Turns any pending GPU error into an `Err`.
    pub fn check(&self) -> Result<()> {
        let errs = self.take_errors();
        if errs.is_empty() {
            Ok(())
        } else {
            Err(anyhow!("GPU error: {}", errs.join("; ")))
        }
    }

    /// Copies `size` bytes of `src` (which must have COPY_SRC) into a staging
    /// buffer, waits for the GPU, and returns the bytes.
    pub fn read_buffer(&self, src: &wgpu::Buffer, size: u64) -> Result<Vec<u8>> {
        self.read_buffer_range(src, 0, size)
    }

    /// Copies just the requested range, avoiding full-image readbacks when
    /// updating the small guiding banks appended to the depth buffer.
    pub fn read_buffer_range(&self, src: &wgpu::Buffer, offset: u64, size: u64) -> Result<Vec<u8>> {
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("syfrack staging"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(src, offset, &staging, 0, size);
        self.queue.submit([enc.finish()]);

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).ok();
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| anyhow!("device poll: {e:?}"))?;
        rx.recv()
            .context("map callback dropped")?
            .context("map_async")?;
        let data = slice
            .get_mapped_range()
            .context("get_mapped_range")?
            .to_vec();
        staging.unmap();
        Ok(data)
    }

    /// Runs the probe shader on 0..256 and returns the results (2*i + 1).
    pub fn run_probe(&self) -> Result<Vec<u32>> {
        let input: Vec<u32> = (0..256).collect();
        let size = (input.len() * 4) as u64;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probe"),
            size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue
            .write_buffer(&buffer, 0, bytemuck::cast_slice(&input));

        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("probe"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/probe.wgsl").into()),
            });
        let bgl = self
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("probe"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("probe"),
                bind_group_layouts: &[Some(&bgl)],
                ..Default::default()
            });
        let pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("probe"),
                layout: Some(&layout),
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("probe"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups((input.len() as u32).div_ceil(64), 1, 1);
        }
        self.queue.submit([enc.finish()]);

        let bytes = self.read_buffer(&buffer, size)?;
        Ok(bytemuck::pod_collect_to_vec(&bytes))
    }
}
