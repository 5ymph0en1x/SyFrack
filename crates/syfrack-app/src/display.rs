use anyhow::{anyhow, Result};
use bytemuck::{Pod, Zeroable};
use syfrack_core::{Look, Tonemap};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    size: [u32; 2],
    art: [f32; 2],
    scalars: [f32; 4],
    agx: [f32; 4],
    balance: [f32; 4],
    shadows: [f32; 4],
    midtones: [f32; 4],
    highlights: [f32; 4],
    vignette: [f32; 4],
}

fn params(size: (u32, u32), look: &Look, exposure_gain: f32) -> Params {
    let g = syfrack_color::balance::gains(look.temperature, look.tint);
    let (kind, agx) = match look.tonemap {
        Tonemap::Direct => (2.0, [1.0, 1.0, 0.0, 1.0]),
        Tonemap::Agx {
            slope,
            power,
            offset,
        } => (0.0, [slope, power, offset, 1.0]),
        Tonemap::Reinhard { white } => (1.0, [1.0, 1.0, 0.0, white]),
    };
    let zone = |z: &syfrack_core::Zone| [z.offset[0], z.offset[1], z.offset[2], z.gain];
    let vignette = match &look.vignette {
        Some(v) => [
            v.radius,
            v.softness,
            v.intensity,
            if v.elliptical { 1.0 } else { 0.0 },
        ],
        None => [0.0; 4],
    };
    Params {
        size: [size.0, size.1],
        art: [look.neon_softness, 0.0],
        scalars: [exposure_gain, look.saturation, look.vibrance, kind],
        agx,
        balance: [g[0], g[1], g[2], size.0 as f32 / size.1 as f32],
        shadows: zone(&look.shadows),
        midtones: zone(&look.midtones),
        highlights: zone(&look.highlights),
        vignette,
    }
}

pub struct DisplayPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    target: Option<(wgpu::Texture, wgpu::TextureView, (u32, u32))>,
    egui_id: Option<egui::TextureId>,
}

impl DisplayPass {
    pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

    pub fn new(device: &wgpu::Device) -> Result<Self> {
        let source = format!(
            "{}\n{}",
            syfrack_color::wgsl_constants(),
            include_str!("shaders/display.wgsl")
        );
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("display"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("display"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Uniform),
                entry(1, wgpu::BufferBindingType::Storage { read_only: true }),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("display"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("display"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: Self::FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(anyhow!("display shader validation failed: {e}"));
        }
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("display params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            pipeline,
            layout,
            params,
            target: None,
            egui_id: None,
        })
    }

    /// (Re)creates the sRGB target at `size` when needed and returns its egui texture id.
    pub fn ensure_target(
        &mut self,
        device: &wgpu::Device,
        egui: &mut egui_wgpu::Renderer,
        size: (u32, u32),
    ) -> egui::TextureId {
        if let (Some((_, _, s)), Some(id)) = (&self.target, self.egui_id) {
            if *s == size {
                return id;
            }
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview target"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
        });
        let view = texture.create_view(&Default::default());
        // egui expects gamma-encoded texels and linearises them itself, so it
        // gets a raw (non-sRGB) view of the bytes the display pass wrote.
        let raw_view = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8Unorm),
            ..Default::default()
        });
        let id = match self.egui_id {
            Some(id) => {
                egui.update_egui_texture_from_wgpu_texture(
                    device,
                    &raw_view,
                    wgpu::FilterMode::Linear,
                    id,
                );
                id
            }
            None => egui.register_native_texture(device, &raw_view, wgpu::FilterMode::Linear),
        };
        self.target = Some((texture, view, size));
        self.egui_id = Some(id);
        id
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        accum: &wgpu::Buffer,
        size: (u32, u32),
        look: &Look,
        exposure_gain: f32,
    ) {
        let Some((_, view, _)) = &self.target else {
            return;
        };
        let view = view.clone();
        self.encode_to(
            device,
            queue,
            encoder,
            accum,
            size,
            look,
            exposure_gain,
            &view,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode_to(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        accum: &wgpu::Buffer,
        size: (u32, u32),
        look: &Look,
        exposure_gain: f32,
        target: &wgpu::TextureView,
    ) {
        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&params(size, look, exposure_gain)),
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("display"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: accum.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("display"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}
