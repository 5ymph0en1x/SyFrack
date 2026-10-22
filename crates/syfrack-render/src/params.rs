//! Keep ordinary parameters in a small uniform prefix. Overflow lives in a
//! read-only texture, avoiding uniform limits and additional storage bindings.
use anyhow::{ensure, Result};

pub(crate) const UNIFORM_SCALARS: usize = 8192;

pub(crate) struct Parameters {
    uniform: wgpu::Buffer,
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl Parameters {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, values: &[f32]) -> Result<Self> {
        let texels = values
            .len()
            .saturating_sub(UNIFORM_SCALARS)
            .div_ceil(4)
            .max(1) as u32;
        let limit = device.limits().max_texture_dimension_2d;
        let width = texels.div_ceil(limit).max(256).min(texels).min(limit);
        let height = texels.div_ceil(width);
        ensure!(
            height <= limit,
            "graph parameter texture exceeds device dimensions"
        );
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("graph parameters"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("graph parameter uniform prefix"),
            size: (UNIFORM_SCALARS * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view = texture.create_view(&Default::default());
        let params = Self {
            uniform,
            texture,
            view,
            width,
            height,
        };
        params.write(queue, values)?;
        Ok(params)
    }

    pub fn binding(&self) -> wgpu::BindingResource<'_> {
        self.uniform.as_entire_binding()
    }

    pub fn overflow_binding(&self) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::TextureView(&self.view)
    }

    pub fn write(&self, queue: &wgpu::Queue, values: &[f32]) -> Result<()> {
        let capacity = self.width as usize * self.height as usize * 4;
        ensure!(
            values.len() <= capacity + UNIFORM_SCALARS,
            "graph parameter layout grew without rebuilding its resources"
        );
        let split = values.len().min(UNIFORM_SCALARS);
        let mut prefix = vec![0.0f32; UNIFORM_SCALARS];
        prefix[..split].copy_from_slice(&values[..split]);
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&prefix));
        let values = &values[split..];
        let mut padded = vec![0.0f32; capacity];
        padded[..values.len()].copy_from_slice(values);
        queue.write_texture(
            self.texture.as_image_copy(),
            bytemuck::cast_slice(&padded),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width * 16),
                rows_per_image: Some(self.height),
            },
            self.texture.size(),
        );
        Ok(())
    }
}
