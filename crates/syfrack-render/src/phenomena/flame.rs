//! A GPU source prepass, not a renderer: the immutable atlas feeds the same
//! typed field operations, live preview, material and final transport.
use super::PreparationControl;
use crate::Renderer;
use anyhow::{ensure, Result};
use syfrack_core::{scene::phenomena::*, FlameTransform, FlameVariation};
use wgpu::util::DeviceExt;

const SOURCE: &str = include_str!("../shaders/flame_field.wgsl");
const LANES: u32 = 4096;
pub struct Engine {
    layout: wgpu::BindGroupLayout,
    pipelines: [wgpu::ComputePipeline; 3],
    cache: std::sync::Mutex<std::collections::VecDeque<(FlameField, FrozenSnapshot)>>,
}
impl Engine {
    pub fn new(r: &Renderer) -> Result<Self> {
        r.with_scopes(|| {
            let d = &r.context().device;
            let entries: Vec<_> = (0..5)
                .map(|binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: if binding == 0 {
                            wgpu::BufferBindingType::Uniform
                        } else {
                            wgpu::BufferBindingType::Storage {
                                read_only: binding == 1,
                            }
                        },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                })
                .collect();
            let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("creation flame source"),
                entries: &entries,
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("creation flame source"),
                bind_group_layouts: &[Some(&layout)],
                ..Default::default()
            });
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("fixed GPU flame source"),
                source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
            });
            let pipelines = ["initialize", "accumulate", "resolve"].map(|entry| {
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pl),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            });
            Ok(Self {
                layout,
                pipelines,
                cache: Default::default(),
            })
        })
    }
}
fn encode(f: &FlameField) -> Vec<[f32; 4]> {
    let mut data = vec![[0.0; 4]; 5];
    let mut transforms: Vec<&FlameTransform> = f.flame.transforms.iter().collect();
    let final_index = f.flame.final_transform.as_ref().map(|t| {
        let i = transforms.len();
        transforms.push(t);
        i
    });
    data.resize(5 + transforms.len() * 5, [0.0; 4]);
    let weight = f.flame.transforms.iter().map(|t| t.weight).sum::<f32>();
    let mut cumulative = 0.0;
    for (i, t) in transforms.into_iter().enumerate() {
        cumulative += t.weight / weight;
        let at = 5 + i * 5;
        data[at] = [cumulative, t.color, t.color_speed, t.terms.len() as f32];
        data[at + 1] = [t.affine[0], t.affine[1], t.affine[2], t.affine[3]];
        data[at + 2] = [t.affine[4], t.affine[5], t.post[0], t.post[1]];
        data[at + 3] = [t.post[2], t.post[3], t.post[4], t.post[5]];
        data[at + 4][0] = data.len() as f32;
        for term in &t.terms {
            let (op, a, b) = match term.variation {
                FlameVariation::Linear => (0, 0.0, 0.0),
                FlameVariation::Sinusoidal => (1, 0.0, 0.0),
                FlameVariation::Spherical => (2, 0.0, 0.0),
                FlameVariation::Swirl => (3, 0.0, 0.0),
                FlameVariation::Horseshoe => (4, 0.0, 0.0),
                FlameVariation::Disc => (5, 0.0, 0.0),
                FlameVariation::Bubble => (6, 0.0, 0.0),
                FlameVariation::Julian { power, distance } => (7, power as f32, distance),
                FlameVariation::Curl { c1, c2 } => (8, c1, c2),
            };
            data.push([op as f32, term.amount, a, b]);
        }
    }
    let palette = data.len();
    for i in 0..4096 {
        let rgb = f.flame.palette.sample(i as f32 / 4095.0);
        data.push([rgb[0], rgb[1], rgb[2], 0.0]);
    }
    let (sn, cs) = f.angle.to_radians().sin_cos();
    data[0] = [
        f.flame.transforms.len() as f32,
        final_index.map_or(-1.0, |i| i as f32),
        0.0,
        0.0,
    ];
    data[1] = [f.center[0], f.center[1], f.half[0], f.half[1]];
    data[2] = [sn, cs, f.flame.brightness, f.flame.density_log];
    data[3] = [
        f.flame.filter_radius,
        f.flame.background[0],
        f.flame.background[1],
        f.flame.background[2],
    ];
    data[4] = [palette as f32, 5.0, 0.0, 0.0];
    data
}
pub fn prepare(
    r: &Renderer,
    engine: &Engine,
    f: &FlameField,
    control: Option<&PreparationControl>,
) -> Result<FrozenSnapshot> {
    f.validate()?;
    if let Some(frozen) = &f.frozen {
        return Ok(frozen.clone());
    }
    if let Some((_, cached)) = engine
        .cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(key, _)| key == f)
    {
        return Ok(cached.clone());
    }
    let ctx = r.context();
    let d = &ctx.device;
    let bytes = u64::from(f.resolution).pow(2) * 16;
    ensure!(
        bytes * 2 <= u64::from(d.limits().max_storage_buffer_binding_size)
            && bytes * 2 <= d.limits().max_buffer_size,
        "flame atlas exceeds GPU binding limit"
    );
    let data = encode(f);
    let buffers = r.with_scopes(|| {
        let make = |label, size, usage| {
            d.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        Ok([
            make(
                "flame source schedule",
                32,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("flame source constants"),
                contents: bytemuck::cast_slice(&data),
                usage: wgpu::BufferUsages::STORAGE,
            }),
            make("GPU orbit states", u64::from(LANES) * 16, storage),
            make("integer orbit moments", bytes * 2, storage),
            make("flame field atlas", bytes, storage),
        ])
    })?;
    let entries: Vec<_> = buffers
        .iter()
        .enumerate()
        .map(|(i, b)| wgpu::BindGroupEntry {
            binding: i as u32,
            resource: b.as_entire_binding(),
        })
        .collect();
    let bind = r.with_scopes(|| {
        Ok(d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flame resource"),
            layout: &engine.layout,
            entries: &entries,
        }))
    })?;
    let rounds = f.points.div_ceil(LANES) + 64;
    let mut begin = 0;
    while begin < rounds {
        if let Some(c) = control {
            c.checkpoint()?;
        }
        let info = [
            f.resolution,
            f.points,
            LANES,
            (f.seed ^ (f.seed >> 32)) as u32,
            begin,
            256,
            0,
            0,
        ];
        ctx.queue
            .write_buffer(&buffers[0], 0, bytemuck::cast_slice(&info));
        let mut encoder = d.create_command_encoder(&Default::default());
        if begin == 0 {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&engine.pipelines[0]);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(LANES / 64, 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&engine.pipelines[1]);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(LANES / 64, 1, 1);
        }
        ctx.queue.submit([encoder.finish()]);
        ctx.read_buffer(&buffers[2], 16)?;
        begin += 256;
    }
    if let Some(c) = control {
        c.checkpoint()?;
    }
    for y in (0..f.resolution).step_by(64) {
        if let Some(c) = control {
            c.checkpoint()?;
        }
        let info = [
            f.resolution,
            f.points,
            LANES,
            0,
            0,
            0,
            y,
            (f.resolution - y).min(64),
        ];
        ctx.queue
            .write_buffer(&buffers[0], 0, bytemuck::cast_slice(&info));
        let mut encoder = d.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&engine.pipelines[2]);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(f.resolution.div_ceil(8), 8, 1);
        }
        ctx.queue.submit([encoder.finish()]);
        ctx.read_buffer(&buffers[2], 16)?;
    }
    let raw = ctx.read_buffer(&buffers[4], bytes)?;
    let values: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&raw);
    ensure!(
        values.iter().flatten().all(|v| v.is_finite() && *v >= 0.0),
        "nonfinite GPU flame source"
    );
    ctx.check()?;
    let frozen = FrozenSnapshot {
        resolution: [f.resolution, f.resolution, 1],
        values: values.into(),
    };
    let mut cache = engine.cache.lock().unwrap_or_else(|e| e.into_inner());
    cache.push_back((f.clone(), frozen.clone()));
    while cache.len() > 2 {
        cache.pop_front();
    }
    Ok(frozen)
}

#[cfg(test)]
mod tests {
    #[test]
    fn shader_validates() {
        let source = super::SOURCE;
        let m = wgpu::naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&m)
        .unwrap();
    }
}
