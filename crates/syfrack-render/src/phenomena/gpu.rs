//! GPU snapshot evolution. Initialization/reference diagnostics stay on the host;
//! every PDE stencil and RK stage executes on compute units.
use super::{
    grid::Grid,
    simulation::{self, Diagnostics},
};
use crate::Renderer;
use anyhow::{ensure, Result};
use bytemuck::{Pod, Zeroable};
use glam::{Vec3, Vec4};
use std::sync::Arc;
use syfrack_core::{scene::phenomena::*, Scene};

const SOURCE: &str = include_str!("../shaders/phenomena_compute.wgsl");
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Info {
    size: [u32; 4],
    scale: [f32; 4],
    reaction: [f32; 4],
    wave: [f32; 4],
}
pub struct Engine {
    layout: wgpu::BindGroupLayout,
    pipelines: [wgpu::ComputePipeline; 4],
}
impl Engine {
    pub fn new(r: &Renderer) -> Result<Self> {
        r.with_scopes(|| {
            let d = &r.context().device;
            let mut entries = vec![];
            for binding in 0..7 {
                entries.push(wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: if binding == 0 {
                            wgpu::BufferBindingType::Uniform
                        } else {
                            wgpu::BufferBindingType::Storage {
                                read_only: binding == 5,
                            }
                        },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                });
            }
            let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("natural PDE"),
                entries: &entries,
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("natural PDE"),
                bind_group_layouts: &[Some(&layout)],
                ..Default::default()
            });
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("precompiled natural PDE"),
                source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
            });
            let pipelines = ["rk1", "rk2", "rk3", "rk4"].map(|entry| {
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pl),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            });
            Ok(Self { layout, pipelines })
        })
    }
}
pub fn simulate(
    r: &Renderer,
    engine: &Arc<Engine>,
    scene: &Scene,
    p: &Phenomena,
    index: usize,
    previous: &[Grid],
    control: Option<&super::PreparationControl>,
) -> Result<(Grid, Diagnostics)> {
    let s = &p.simulations[index];
    let wave = matches!(s.model, SimulationModel::GrossPitaevskii { .. });
    let mut grid = Grid::zero(&p.domain);
    simulation::seed(scene, p, s, &mut grid, wave);
    let coefficients = simulation::coefficients(scene, p, s, previous, &grid)?;
    for (v, c) in grid.data.iter_mut().zip(&coefficients) {
        if wave && c.z <= 0.0 {
            *v = Vec4::ZERO;
        }
    }
    let mut diagnostics = Diagnostics {
        mass_before: simulation::mass(&grid),
        ..Default::default()
    };
    let h = grid.spacing();
    let inverse = (Vec3::ONE / (h * h)).to_array();
    let sum = (0..3)
        .filter(|&a| p.domain.resolution[a] > 1)
        .map(|a| inverse[a])
        .sum::<f32>();
    let rsize = p.domain.resolution;
    let mut info = Info {
        size: [rsize[0], rsize[1], rsize[2], u32::from(wave)],
        scale: [inverse[0], inverse[1], inverse[2], 0.0],
        reaction: [0.0; 4],
        wave: [0.0; 4],
    };
    match s.model {
        SimulationModel::GrayScott {
            diffusion,
            feed,
            kill,
        } => info.reaction = [diffusion[0], diffusion[1], feed, kill],
        SimulationModel::GrossPitaevskii {
            dispersion,
            interaction,
            damping,
        } => info.wave = [dispersion, interaction, damping, 0.0],
        _ => unreachable!(),
    }
    let ctx = r.context();
    let d = &ctx.device;
    let bytes = grid.data.len() as u64 * 16;
    ensure!(
        bytes <= u64::from(d.limits().max_storage_buffer_binding_size),
        "PDE grid exceeds GPU storage binding limit"
    );
    let buffers = r.with_scopes(|| {
        let make = |label, size, usage| {
            d.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let storage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC;
        Ok([
            make(
                "PDE info",
                64,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
            make("PDE initial", bytes, storage),
            make("PDE stage A", bytes, storage),
            make("PDE stage B", bytes, storage),
            make("PDE RK sum", bytes, storage),
            make("PDE coefficients", bytes, storage),
            make("PDE statistics", 16, storage),
        ])
    })?;
    let rows: Vec<_> = grid.data.iter().map(|v| v.to_array()).collect();
    let coeff_rows: Vec<_> = coefficients.iter().map(|v| v.to_array()).collect();
    ctx.queue
        .write_buffer(&buffers[1], 0, bytemuck::cast_slice(&rows));
    ctx.queue
        .write_buffer(&buffers[5], 0, bytemuck::cast_slice(&coeff_rows));
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
            label: Some("PDE snapshot"),
            layout: &engine.layout,
            entries: &entries,
        }))
    })?;
    let mut max_rho = grid
        .data
        .iter()
        .map(|v| v.x * v.x + v.y * v.y)
        .fold(0.0, f32::max);
    for _ in 0..s.steps {
        if let Some(c) = control {
            c.checkpoint()?;
        }
        let bound = match s.model {
            SimulationModel::GrayScott { diffusion, .. } => {
                2.0 * diffusion[0].max(diffusion[1]) * sum + 4.0
            }
            SimulationModel::GrossPitaevskii {
                dispersion,
                interaction,
                ..
            } => {
                4.0 * dispersion * sum
                    + interaction.abs() * max_rho
                    + coefficients.iter().map(|c| c.x.abs()).fold(0.0, f32::max)
            }
            _ => unreachable!(),
        };
        let subdivisions = (s.dt * bound / 0.2).ceil().max(1.0) as u32;
        ensure!(subdivisions <= 4096, "PDE stable substep budget exceeded");
        ensure!(
            (diagnostics.substeps + u64::from(subdivisions)) * grid.data.len() as u64
                <= 1_000_000_000,
            "PDE snapshot work budget exceeded"
        );
        info.scale[3] = s.dt / subdivisions as f32;
        ctx.queue
            .write_buffer(&buffers[0], 0, bytemuck::bytes_of(&info));
        ctx.queue.write_buffer(&buffers[6], 0, &[0u8; 16]);
        let batch = (1_000_000usize / grid.data.len()).clamp(1, 32) as u32;
        let mut done = 0;
        while done < subdivisions {
            if let Some(c) = control {
                c.checkpoint()?;
            }
            let end = (done + batch).min(subdivisions);
            let mut encoder = d.create_command_encoder(&Default::default());
            for _ in done..end {
                for pipeline in &engine.pipelines {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &bind, &[]);
                    pass.dispatch_workgroups((grid.data.len() as u32).div_ceil(64), 1, 1);
                }
            }
            ctx.queue.submit([encoder.finish()]);
            let stats = ctx.read_buffer(&buffers[6], 16)?;
            let words: Vec<u32> = bytemuck::pod_collect_to_vec(&stats);
            ensure!(words[1] == 0, "GPU PDE produced non-finite state");
            max_rho = f32::from_bits(words[0]);
            done = end;
        }
        diagnostics.substeps += u64::from(subdivisions);
    }
    let raw = ctx.read_buffer(&buffers[1], bytes)?;
    let rows: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&raw);
    grid.data = rows.into_iter().map(Vec4::from_array).collect();
    diagnostics.mass_after = simulation::mass(&grid);
    if wave {
        for v in &mut grid.data {
            v.z = v.x * v.x + v.y * v.y;
            v.w = v.y.atan2(v.x) / std::f32::consts::TAU + 0.5;
        }
    }
    ctx.check()?;
    Ok((grid, diagnostics))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_pde_shader_validates() {
        let m = wgpu::naga::front::wgsl::parse_str(SOURCE)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(SOURCE)));
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&m)
        .unwrap();
    }
}
