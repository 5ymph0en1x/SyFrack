//! Interactive geometry, independent of graph-specific shader compilation.
//! A fixed GPU machine executes exact graph operations in bounded quanta.
//! Editing uploads data; the UI never waits for a dispatch or a full image.
mod program;

use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use anyhow::{ensure, Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use syfrack_core::{DeSample, Scene};

use crate::{Renderer, Uniforms};
use program::{Program, OPS};

const MEMORY_BUDGET: u64 = 64 * 1024 * 1024;
const QUANTUM: u32 = 128;

fn source() -> String {
    let common = crate::shader_source::COMMON;
    // Reuse the production graph primitives verbatim, without its transport,
    // generated evaluators, textures, lighting or material entry points.
    let helpers = common.split("\nfn de(p:").next().unwrap();
    let orbit = common.split("\nfn orbit_angle(").nth(1).unwrap();
    let mut text = String::from(helpers);
    let colors_start = common.find("fn palette_mix(").unwrap();
    let colors_end = common.find("// A zero flag certifies").unwrap();
    text.push_str(&common[colors_start..colors_end]);
    text.push_str(crate::shader_source::PHENOMENA);
    text.push_str(crate::shader_source::PLANAR);
    text.push_str(crate::shader_source::PATTERN);
    text.push_str(crate::shader_source::CREATION);
    text.push_str(
        "\nfn texture_de_sample(p:vec3f)->DeSample {return DeSample(0.0,vec4f(1.0),0u,0u,0u);}\n",
    );
    text.push_str(crate::shader_source::GEODESIC);
    text.push_str("\nfn orbit_angle(");
    text.push_str(orbit);
    text.push_str(include_str!("../shaders/cubic_polar.wgsl"));
    for op in OPS {
        text.push_str(&format!("\nconst OP_{op:?}:u32={}u;", *op as u32));
    }
    text.push_str(include_str!("../shaders/control.wgsl"));
    text
}

pub(crate) struct ControlEngine {
    layout: wgpu::BindGroupLayout,
    init: wgpu::ComputePipeline,
    step: wgpu::ComputePipeline,
}

impl ControlEngine {
    pub(crate) fn new(renderer: &Renderer) -> Result<Self> {
        renderer
            .with_scopes(|| {
                let device = &renderer.context().device;
                let buffer = |binding, ty| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                };
                let uniform = wgpu::BufferBindingType::Uniform;
                let storage = wgpu::BufferBindingType::Storage { read_only: false };
                let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("control machine"),
                    entries: &[
                        buffer(0, uniform),
                        buffer(1, wgpu::BufferBindingType::Storage { read_only: true }),
                        buffer(2, storage),
                        buffer(3, storage),
                        buffer(4, uniform),
                        buffer(5, storage),
                        buffer(6, storage),
                        buffer(7, storage),
                        buffer(8, uniform),
                        wgpu::BindGroupLayoutEntry {
                            binding: 16,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                    ],
                });
                let pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("control machine"),
                        bind_group_layouts: &[Some(&layout)],
                        ..Default::default()
                    });
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("fixed graph control"),
                    source: wgpu::ShaderSource::Wgsl(source().into()),
                });
                let pipeline = |entry| {
                    let clock = Instant::now();
                    if std::env::var_os("SYFRACK_TRACE_GPU").is_some() {
                        eprintln!("compiling {entry}");
                    }
                    let result = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: Some(&pipeline_layout),
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: None,
                    });
                    if std::env::var_os("SYFRACK_TRACE_GPU").is_some() {
                        eprintln!("compiled {entry} in {:?}", clock.elapsed());
                    }
                    result
                };
                Ok(Self {
                    init: pipeline("control_init"),
                    step: pipeline("control_step"),
                    layout,
                })
            })
            .context("preparing fixed GPU control engine")
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Info {
    program: [u32; 4],
    config: [u32; 4],
}

struct Pending {
    receiver: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    submission: wgpu::SubmissionIndex,
}

pub struct ControlSession {
    renderer: Renderer,
    engine: Arc<ControlEngine>,
    program: Program,
    requested_size: (u32, u32),
    size: (u32, u32),
    mode: u32,
    uniform: wgpu::Buffer,
    params: crate::params::Parameters,
    instructions: wgpu::Buffer,
    _scratch: wgpu::Buffer,
    accum: wgpu::Buffer,
    depth: wgpu::Buffer,
    aux: wgpu::Buffer,
    status: wgpu::Buffer,
    staging: wgpu::Buffer,
    _info: wgpu::Buffer,
    bind: wgpu::BindGroup,
    pending: Option<Pending>,
    initialize: bool,
    remaining: u32,
    failure: Option<String>,
    dispatches: u32,
    started: Instant,
}

impl ControlSession {
    pub fn new(renderer: Renderer, scene: &Scene, size: (u32, u32)) -> Result<Self> {
        Self::build(renderer, scene, size, 0)
    }

    fn build(
        renderer: Renderer,
        scene: &Scene,
        requested_size: (u32, u32),
        mode: u32,
    ) -> Result<Self> {
        let resolved = scene.render_scene()?;
        let scene = &resolved;
        syfrack_core::params::validate_params(&scene.camera, "camera")?;
        let program = Program::compile(scene.fractal.graph())?;
        let engine = renderer.control_engine()?;
        let device = &renderer.context().device;
        let queue = &renderer.context().queue;
        let stride = u64::from(program.registers) * 3 + 10 + u64::from(program.frames) * 2;
        let budget = MEMORY_BUDGET
            .min(device.limits().max_storage_buffer_binding_size.into())
            .min(device.limits().max_buffer_size);
        let max_pixels = (budget / (stride * 16)).min(u32::MAX as u64);
        ensure!(
            max_pixels >= 1,
            "control register memory exceeds the device limit"
        );
        let (rw, rh) = (requested_size.0.max(1), requested_size.1.max(1));
        let factor = (max_pixels as f64 / (u64::from(rw) * u64::from(rh)) as f64)
            .sqrt()
            .min(1.0);
        let mut size = (
            (rw as f64 * factor).floor().max(1.0) as u32,
            (rh as f64 * factor).floor().max(1.0) as u32,
        );
        if u64::from(size.0) * u64::from(size.1) > max_pixels {
            if size.0 >= size.1 {
                size.0 = (max_pixels / u64::from(size.1)).max(1) as u32;
            } else {
                size.1 = (max_pixels / u64::from(size.0)).max(1) as u32;
            }
        }
        ensure!(
            mode == 0 || size == (rw, rh),
            "control probe batch exceeds scratch memory"
        );
        let pixels = size.0 * size.1;
        ensure!(
            u64::from(pixels) * stride <= u32::MAX as u64,
            "control scratch indexing overflow"
        );
        let storage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC;
        let make = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let mut uniforms = Uniforms::from_scene(scene, size.0, size.1, 0, u32::MAX);
        uniforms.fractal.p0[0] = 1.0;
        let uniform = make(
            "control uniforms",
            std::mem::size_of::<Uniforms>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let prepared = renderer.prepare_phenomena(scene)?;
        let mut values = program.layout.pack(scene.fractal.graph());
        crate::phenomena::append_parameters(
            scene,
            prepared.as_deref(),
            &mut values,
            &mut uniforms,
        )?;
        let params = crate::params::Parameters::new(device, queue, &values)?;
        let instructions = make(
            "control instructions",
            (program.instructions.len() * 32) as u64,
            storage,
        );
        let scratch = make(
            "control SoA registers",
            u64::from(pixels) * stride * 16,
            storage,
        );
        let accum = make("control image", u64::from(pixels) * 16, storage);
        let depth = make("control depth", u64::from(pixels) * 4, storage);
        let aux = make("control normals", u64::from(pixels) * 16, storage);
        let status = make("control status", 16, storage);
        let staging = Self::staging(device);
        let info = make(
            "control program header",
            32,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        queue.write_buffer(&uniform, 0, bytemuck::bytes_of(&uniforms));
        queue.write_buffer(
            &instructions,
            0,
            bytemuck::cast_slice(&program.instructions),
        );
        queue.write_buffer(
            &info,
            0,
            bytemuck::bytes_of(&Info {
                program: [
                    program.registers,
                    program.frames,
                    program.instructions.len() as u32,
                    QUANTUM,
                ],
                config: [mode, pixels, stride as u32, 0],
            }),
        );
        fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: buffer.as_entire_binding(),
            }
        }
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("control scene"),
            layout: &engine.layout,
            entries: &[
                entry(0, &uniform),
                entry(1, &instructions),
                entry(2, &scratch),
                entry(3, &accum),
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: params.binding(),
                },
                entry(5, &depth),
                entry(6, &aux),
                entry(7, &status),
                entry(8, &info),
                wgpu::BindGroupEntry {
                    binding: 16,
                    resource: params.overflow_binding(),
                },
            ],
        });
        Ok(Self {
            renderer,
            engine,
            program,
            requested_size,
            size,
            mode,
            uniform,
            params,
            instructions,
            _scratch: scratch,
            accum,
            depth,
            aux,
            status,
            staging,
            _info: info,
            bind,
            pending: None,
            initialize: true,
            remaining: pixels,
            failure: None,
            dispatches: 0,
            started: Instant::now(),
        })
    }

    fn staging(device: &wgpu::Device) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("control async status"),
            size: 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Upload changes without any shader creation. Compatible register layouts
    /// reuse their buffers. A new topology is committed only after validation.
    pub fn set_scene(&mut self, scene: &Scene) -> Result<()> {
        let resolved = scene.render_scene()?;
        let scene = &resolved;
        syfrack_core::params::validate_params(&scene.camera, "camera")?;
        let program = Program::compile(scene.fractal.graph())?;
        let compatible = program.registers == self.program.registers
            && program.frames == self.program.frames
            && program.instructions.len() == self.program.instructions.len()
            && program.layout.len() == self.program.layout.len();
        if !compatible {
            *self = Self::build(self.renderer.clone(), scene, self.requested_size, self.mode)?;
            return Ok(());
        }
        let queue = &self.renderer.context().queue;
        let mut uniforms = Uniforms::from_scene(scene, self.size.0, self.size.1, 0, u32::MAX);
        uniforms.fractal.p0[0] = 1.0;
        let prepared = self.renderer.prepare_phenomena(scene)?;
        let mut values = program.layout.pack(scene.fractal.graph());
        crate::phenomena::append_parameters(
            scene,
            prepared.as_deref(),
            &mut values,
            &mut uniforms,
        )?;
        if self.params.write(queue, &values).is_err() {
            *self = Self::build(self.renderer.clone(), scene, self.requested_size, self.mode)?;
            return Ok(());
        }
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniforms));
        queue.write_buffer(
            &self.instructions,
            0,
            bytemuck::cast_slice(&program.instructions),
        );
        // A pending callback belongs to the old image. Cancel its mapping and
        // use a fresh tiny staging buffer; queued GPU commands retain resources.
        if self.pending.take().is_some() {
            self.staging.unmap();
            self.staging = Self::staging(&self.renderer.context().device);
        }
        self.program = program;
        self.initialize = true;
        self.remaining = self.size.0 * self.size.1;
        self.failure = None;
        self.dispatches = 0;
        self.started = Instant::now();
        Ok(())
    }

    /// Poll once and submit at most one instruction quantum. Never waits for GPU.
    pub fn tick(&mut self) -> Result<()> {
        if let Some(error) = &self.failure {
            anyhow::bail!("{error}");
        }
        let device = &self.renderer.context().device;
        device
            .poll(wgpu::PollType::Poll)
            .context("poll control GPU")?;
        if let Some(pending) = &self.pending {
            match pending.receiver.try_recv() {
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
                Err(e) => return Err(e).context("control status callback dropped"),
                Ok(result) => result.context("map control status")?,
            }
            let bytes = self.staging.slice(..).get_mapped_range()?;
            let status: [u32; 4] = bytemuck::pod_read_unaligned(&bytes);
            drop(bytes);
            self.staging.unmap();
            self.pending = None;
            if status[2] != 0 {
                let error=format!("control graph produced a non-finite distance or invalid execution near node {}",status[3]);
                self.failure = Some(error.clone());
                anyhow::bail!(error);
            }
            self.remaining = status[0];
            self.dispatches += 1;
        }
        if self.remaining == 0 {
            return Ok(());
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("control quantum"),
        });
        encoder.clear_buffer(&self.status, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("control quantum"),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.bind, &[]);
            if self.initialize {
                pass.set_pipeline(&self.engine.init);
                pass.dispatch_workgroups((self.size.0 * self.size.1).div_ceil(64), 1, 1);
                self.initialize = false;
            }
            pass.set_pipeline(&self.engine.step);
            pass.dispatch_workgroups((self.size.0 * self.size.1).div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&self.status, 0, &self.staging, 0, 16);
        let submission = self.renderer.context().queue.submit([encoder.finish()]);
        let (tx, receiver) = mpsc::channel();
        self.staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
        self.pending = Some(Pending {
            receiver,
            submission,
        });
        Ok(())
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }
    pub fn requested_size(&self) -> (u32, u32) {
        self.requested_size
    }
    pub fn accumulation_buffer(&self) -> &wgpu::Buffer {
        &self.accum
    }
    pub fn complete(&self) -> bool {
        self.remaining == 0
    }
    pub fn completed_pixels(&self) -> u32 {
        self.size.0 * self.size.1 - self.remaining
    }
    pub fn dispatches(&self) -> u32 {
        self.dispatches
    }
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
    pub fn pick(&self, u: f32, v: f32) -> Result<Option<(f32, [f32; 4])>> {
        let x = (u.clamp(0.0, 0.9999) * self.size.0 as f32) as u64;
        let y = (v.clamp(0.0, 0.9999) * self.size.1 as f32) as u64;
        let index = y * self.size.0 as u64 + x;
        let ctx = self.renderer.context();
        let depth = ctx.read_buffer_range(&self.depth, index * 4, 4)?;
        let d: f32 = bytemuck::pod_read_unaligned(&depth);
        if d <= 0.0 {
            return Ok(None);
        }
        let aux = ctx.read_buffer_range(&self.aux, index * 16, 16)?;
        Ok(Some((d, bytemuck::pod_read_unaligned(&aux))))
    }
    /// Blocking diagnostic only; interactive clients use tick.
    pub fn wait_complete(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(120);
        while !self.complete() {
            ensure!(Instant::now() < deadline, "control evaluation timed out");
            self.tick()?;
            if let Some(p) = &self.pending {
                self.renderer.context().device.poll(wgpu::PollType::Wait {
                    submission_index: Some(p.submission.clone()),
                    timeout: Some(Duration::from_secs(10)),
                })?;
            }
        }
        self.renderer.context().check()
    }
}

impl Renderer {
    pub(crate) fn control_engine(&self) -> Result<Arc<ControlEngine>> {
        let mut engine = self.control.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(engine) = engine.as_ref() {
            return Ok(engine.clone());
        }
        let built = Arc::new(ControlEngine::new(self)?);
        *engine = Some(built.clone());
        Ok(built)
    }

    /// Probe the fixed interpreter, without compiling a topology-specific shader.
    pub fn control_distances(&self, scene: &Scene, points: &[Vec3]) -> Result<Vec<DeSample>> {
        let mut out = Vec::with_capacity(points.len());
        let plan = Program::compile(scene.fractal.graph())?;
        let bytes_per_point =
            (u64::from(plan.registers) * 3 + 10 + u64::from(plan.frames) * 2) * 16;
        let limit = MEMORY_BUDGET
            .min(
                self.context()
                    .device
                    .limits()
                    .max_storage_buffer_binding_size
                    .into(),
            )
            .min(self.context().device.limits().max_buffer_size);
        let batch = (limit / bytes_per_point).clamp(1, 4096) as usize;
        for points in points.chunks(batch) {
            let mut session =
                ControlSession::build(self.clone(), scene, (points.len() as u32, 1), 1)?;
            let input: Vec<[f32; 4]> = points.iter().map(|p| [p.x, p.y, p.z, 1.0]).collect();
            self.context()
                .queue
                .write_buffer(&session.accum, 0, bytemuck::cast_slice(&input));
            session.wait_complete()?;
            let bytes = self
                .context()
                .read_buffer(&session.accum, (points.len() * 16) as u64)?;
            let traps = self
                .context()
                .read_buffer(&session.aux, (points.len() * 16) as u64)?;
            for (p, t) in bytes.chunks_exact(16).zip(traps.chunks_exact(16)) {
                let p: [f32; 4] = bytemuck::pod_read_unaligned(p);
                out.push(DeSample {
                    distance: p[0],
                    traps: bytemuck::pod_read_unaligned(t),
                    iterations: p[1] as u32,
                    material: p[2] as u32,
                });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fixed_machine_validates_and_generates_both_spirv_entry_points() {
        let source = super::source();
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        let info = wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
        for entry in &module.entry_points {
            wgpu::naga::back::spv::write_vec(
                &module,
                &info,
                &Default::default(),
                Some(&wgpu::naga::back::spv::PipelineOptions {
                    shader_stage: entry.stage,
                    entry_point: entry.name.clone(),
                }),
            )
            .unwrap();
        }
    }
}
