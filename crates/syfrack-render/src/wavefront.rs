//! GPU-resident bounce queues. No host readback or wait between path stages.
use crate::{CompiledPipeline, Renderer};
use anyhow::{anyhow, ensure, Context, Result};
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

const WIDTH: u32 = 64;
const STATE_BYTES: u64 = 19 * 16;
const FILM_BYTES: u64 = 8 * 16;
const TEMPLATE: &str = include_str!("shaders/wavefront.wgsl");
fn binding(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

/// Compile the same bounce operations as the reference, with explicit state
/// transitions instead of an outer per-invocation bounce loop.
fn source(reference: &str) -> Result<String> {
    let normalized = reference.replace("\r\n", "\n");
    let reference = normalized.as_str();
    let (prefix, radiance) = reference
        .split_once("\nfn radiance(")
        .context("radiance entry missing")?;
    let (_, body) = radiance
        .split_once("for (var bounce = 0u; bounce <= u.max_bounces;) {")
        .context("bounce loop missing")?;
    let (body, _) = body
        .split_once("\n    }\n    if (GUIDE_TRAINING)")
        .context("bounce terminator missing")?;
    let body = body
        .replace(
            "var h = trace_scene(ray.o, ray.d, pixel_cone, ray_extent());",
            "var h = wave_hit;",
        )
        .replace("h = optical_refine_hit(ray.o,ray.d,h);", "")
        .replace("continue;", "wf_alive=true; break;");
    ensure!(
        body.contains("var h = wave_hit;") && !body.contains("continue;"),
        "invalid bounce transition"
    );
    let start = prefix
        .find("\n@group(0) @binding(1) var<storage, read_write> accum:")
        .context("film bindings missing")?
        + 1;
    let end = prefix[start..]
        .find("// ---------------------------------------------------------------- RNG")
        .context("RNG section missing")?
        + start;
    let mut out = prefix[..start].to_owned();
    out.push_str(&prefix[end..]);
    out.push_str(&TEMPLATE.replace("$BOUNCE_BODY", &body));
    Ok(out)
}

pub(crate) struct Pipelines {
    scene: wgpu::BindGroupLayout,
    work: wgpu::BindGroupLayout,
    args: wgpu::BindGroupLayout,
    start: wgpu::ComputePipeline,
    generate: wgpu::ComputePipeline,
    prepare: wgpu::ComputePipeline,
    intersect: wgpu::ComputePipeline,
    shade: wgpu::ComputePipeline,
    compact: wgpu::ComputePipeline,
    commit: wgpu::ComputePipeline,
    pub fingerprint: u64,
}

impl Pipelines {
    fn build(device: &wgpu::Device, reference: &str) -> Result<Self> {
        let text = source(reference)?;
        let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let mut entries = Vec::new();
        for binding in [0, 4, 5] {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
        }
        entries.push(storage(2, false));
        entries.push(storage(6, true));
        for binding in [7, 9, 10, 16, 17] {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float {
                        filterable: binding < 16,
                    },
                    view_dimension: if binding < 16 {
                        wgpu::TextureViewDimension::D3
                    } else {
                        wgpu::TextureViewDimension::D2
                    },
                    multisampled: false,
                },
                count: None,
            });
        }
        for binding in [8, 11] {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let scene = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wavefront scene"),
            entries: &entries,
        });
        let work = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wavefront arenas"),
            entries: &[storage(0, false), storage(1, false), storage(2, false)],
        });
        let args = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wavefront indirect arguments"),
            entries: &[storage(0, false)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wavefront"),
            bind_group_layouts: &[Some(&scene), Some(&work)],
            ..Default::default()
        });
        let args_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wavefront argument builder"),
            bind_group_layouts: &[Some(&scene), Some(&work), Some(&args)],
            ..Default::default()
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wavefront transport"),
            source: wgpu::ShaderSource::Wgsl(text.clone().into()),
        });
        let make = |entry, layout| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[("GUIDE_TRAINING", 0.0), ("GUIDE_SAMPLING", 0.0)],
                    ..Default::default()
                },
                cache: None,
            })
        };
        let result = Self {
            start: make("wf_start", &layout),
            generate: make("wf_generate", &layout),
            prepare: make("wf_prepare", &args_layout),
            intersect: make("wf_intersect", &layout),
            shade: make("wf_shade", &layout),
            compact: make("wf_compact", &layout),
            commit: make("wf_commit", &layout),
            fingerprint: crate::shader_source::transport_fingerprint(&text),
            scene,
            work,
            args,
        };
        let validation = pollster::block_on(validation.pop());
        let internal = pollster::block_on(internal.pop());
        let oom = pollster::block_on(oom.pop());
        if let Some(e) = validation.or(internal).or(oom) {
            anyhow::bail!("wavefront pipeline compilation: {e}");
        }
        Ok(result)
    }
}

pub(crate) struct Wavefront {
    pipelines: Arc<Pipelines>,
    scene: wgpu::BindGroup,
    work: wgpu::BindGroup,
    args_group: wgpu::BindGroup,
    state: wgpu::Buffer,
    queue: wgpu::Buffer,
    film: wgpu::Buffer,
    args: wgpu::Buffer,
    status: wgpu::Buffer,
    capacity: u32,
    rounds: u32,
}

impl Wavefront {
    pub fn new(
        renderer: &Renderer,
        pipeline: &Arc<CompiledPipeline>,
        capacity: u64,
        rounds: u32,
        entries: &[wgpu::BindGroupEntry<'_>],
    ) -> Result<Self> {
        let device = &renderer.ctx.device;
        ensure!(
            capacity > 0 && capacity <= u32::MAX as u64,
            "invalid wavefront capacity"
        );
        let sizes = [
            capacity * STATE_BYTES,
            64 + capacity * 8,
            capacity * FILM_BYTES,
            16,
            16,
        ];
        ensure!(
            sizes.iter().sum::<u64>() <= 512 * 1024 * 1024,
            "wavefront working set exceeds 512 MiB; use bounded export tiles"
        );
        for bytes in sizes {
            ensure!(
                bytes <= device.limits().max_buffer_size
                    && bytes <= u64::from(device.limits().max_storage_buffer_binding_size),
                "wavefront arena exceeds device limits"
            );
        }
        let pipelines = {
            let mut cache = pipeline.wavefront.lock().unwrap_or_else(|p| p.into_inner());
            if cache.is_none() {
                *cache = Some(Arc::new(Pipelines::build(device, &pipeline.export_source)?));
            }
            cache.as_ref().unwrap().clone()
        };
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let rw = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST;
        let state = buffer("wavefront SoA paths and hits", sizes[0], rw);
        let queue = buffer("wavefront ping-pong IDs", sizes[1], rw);
        let film = buffer("wavefront film planes", sizes[2], rw);
        let args = buffer(
            "wavefront GPU-generated dispatch",
            16,
            rw | wgpu::BufferUsages::INDIRECT,
        );
        let status = buffer(
            "wavefront completion status",
            16,
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        );
        let scene = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wavefront scene"),
            layout: &pipelines.scene,
            entries,
        });
        let work = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wavefront working set"),
            layout: &pipelines.work,
            entries: &[binding(0, &state), binding(1, &queue), binding(2, &film)],
        });
        let args_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wavefront argument output"),
            layout: &pipelines.args,
            entries: &[binding(0, &args)],
        });
        Ok(Self {
            pipelines,
            scene,
            work,
            args_group,
            state,
            queue,
            film,
            args,
            status,
            capacity: capacity as u32,
            rounds,
        })
    }
    pub fn fingerprint(&self) -> u64 {
        self.pipelines.fingerprint
    }
    pub fn working_bytes(&self) -> u64 {
        self.state.size()
            + self.queue.size()
            + self.film.size()
            + self.args.size()
            + self.status.size()
    }

    /// Film order: accum, aux, volume, moments, surface, then the three packed
    /// volume-component planes. Copies preserve legacy readback/checkpoint ABI.
    pub fn dispatch(
        &self,
        renderer: &Renderer,
        width: u32,
        height: u32,
        samples: u32,
        planes: [&wgpu::Buffer; 6],
        components: bool,
    ) -> Result<Duration> {
        let count = width
            .checked_mul(height)
            .context("wavefront batch overflow")?;
        ensure!(count <= self.capacity, "wavefront batch exceeds capacity");
        let ctx = &renderer.ctx;
        let mut header = [0u32; 16];
        header[0] = self.capacity;
        header[1] = count;
        header[2] = u32::MAX;
        ctx.queue
            .write_buffer(&self.queue, 0, bytemuck::cast_slice(&header));
        let mut enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("wavefront batch"),
                ..Default::default()
            });
        let bytes = u64::from(self.capacity) * 16;
        for (i, b) in planes[..5].iter().enumerate() {
            enc.copy_buffer_to_buffer(b, 0, &self.film, i as u64 * bytes, bytes);
        }
        if components {
            enc.copy_buffer_to_buffer(planes[5], 0, &self.film, 5 * bytes, 3 * bytes);
        }
        let pass = |enc: &mut wgpu::CommandEncoder,
                    pipeline: &wgpu::ComputePipeline,
                    indirect: bool,
                    args: bool,
                    groups: u32| {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("wavefront stage"),
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.scene, &[]);
            pass.set_bind_group(1, &self.work, &[]);
            if args {
                pass.set_bind_group(2, &self.args_group, &[]);
            }
            if indirect {
                pass.dispatch_workgroups_indirect(&self.args, 0);
            } else {
                pass.dispatch_workgroups(groups, 1, 1);
            }
        };
        for _ in 0..samples {
            pass(&mut enc, &self.pipelines.start, false, false, 1);
            pass(
                &mut enc,
                &self.pipelines.generate,
                false,
                false,
                count.div_ceil(WIDTH),
            );
            for _ in 0..self.rounds {
                pass(&mut enc, &self.pipelines.prepare, false, true, 1);
                pass(&mut enc, &self.pipelines.intersect, true, false, 0);
                pass(&mut enc, &self.pipelines.shade, true, false, 0);
                pass(&mut enc, &self.pipelines.compact, true, false, 0);
            }
            pass(
                &mut enc,
                &self.pipelines.commit,
                false,
                false,
                count.div_ceil(WIDTH),
            );
        }
        for (i, b) in planes[..5].iter().enumerate() {
            enc.copy_buffer_to_buffer(&self.film, i as u64 * bytes, b, 0, bytes);
        }
        if components {
            enc.copy_buffer_to_buffer(&self.film, 5 * bytes, planes[5], 0, 3 * bytes);
        }
        enc.copy_buffer_to_buffer(&self.queue, 16, &self.status, 0, 16);
        let started = Instant::now();
        let submission = ctx.queue.submit([enc.finish()]);
        let (tx, rx) = mpsc::channel();
        self.status
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
        ctx.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(10)),
            })
            .map_err(|e| anyhow!("wavefront submission: {e}"))?;
        rx.recv().context("wavefront status callback dropped")??;
        let mapped = self.status.slice(..).get_mapped_range()?;
        let status: Vec<u32> = bytemuck::pod_collect_to_vec(&mapped);
        drop(mapped);
        self.status.unmap();
        ctx.check()?;
        ensure!(
            status[0] == 0 && status[2] == 0,
            "wavefront queue/termination invariant failed: {status:?}"
        );
        Ok(started.elapsed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_wavefront_validates_for_bundled_graphs_and_extended_transports() {
        for &(name, _) in syfrack_core::graph::bundled::BUNDLED {
            let scene = syfrack_core::Scene::example_for(name).unwrap();
            let compiled = crate::graph_wgsl::compile(scene.fractal.graph()).unwrap();
            let reference =
                crate::shader_source::pathtrace_with_features(&compiled.wgsl, None, true, true);
            let source = source(&reference).unwrap();
            let module = wgpu::naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            let info = wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            for entry in &module.entry_points {
                let (module, info) = wgpu::naga::back::pipeline_constants::process_overrides(
                    &module,
                    &info,
                    Some((wgpu::naga::ShaderStage::Compute, &entry.name)),
                    &Default::default(),
                )
                .unwrap();
                wgpu::naga::back::spv::write_vec(
                    &module,
                    &info,
                    &Default::default(),
                    Some(&wgpu::naga::back::spv::PipelineOptions {
                        shader_stage: wgpu::naga::ShaderStage::Compute,
                        entry_point: entry.name.clone(),
                    }),
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn node_comments_cannot_shadow_transport_delimiters() {
        let scene = syfrack_core::Scene::example();
        let mut graph = scene.fractal.to_graph();
        let a = source(&crate::shader_source::pathtrace(
            &crate::graph_wgsl::compile(&graph).unwrap().wgsl,
        ))
        .unwrap();
        for node in &mut graph.nodes {
            node.name = "fn radiance( @group(0) @binding(1) var<storage, read_write> accum:".into();
        }
        let b = source(&crate::shader_source::pathtrace(
            &crate::graph_wgsl::compile(&graph).unwrap().wgsl,
        ))
        .unwrap();
        assert_eq!(
            crate::shader_source::transport_fingerprint(&a),
            crate::shader_source::transport_fingerprint(&b)
        );
    }
}
