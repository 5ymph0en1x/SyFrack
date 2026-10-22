//! Strict, diagnostic CPU/WGSL parity gate. No tolerance can enable this gate.
//! Compile only the distance probe: auditing a graph must not compile a tracer.
use crate::{graph_wgsl, shader_source, Renderer, Uniforms};
use anyhow::{ensure, Context, Result};
use glam::Vec3;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use syfrack_core::{graph::eval::PreparedGraph, DeSample, Graph, Scene};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Difference {
    pub point: [f32; 3],
    /// Distance, four traps, iterations, material. Float entries are raw bits.
    pub cpu: [u32; 7],
    pub gpu: [u32; 7],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphParity {
    pub graph: String,
    pub points: usize,
    pub distance_mismatches: usize,
    pub trap_mismatches: [usize; 4],
    pub iteration_mismatches: usize,
    pub material_mismatches: usize,
    pub nonfinite_distances: usize,
    pub max_distance_ulp: u32,
    pub max_distance_absolute_error: f32,
    pub first_difference: Option<Difference>,
}

impl GraphParity {
    pub fn is_bit_exact(&self) -> bool {
        self.distance_mismatches == 0
            && self.points > 0
            && self.trap_mismatches == [0; 4]
            && self.iteration_mismatches == 0
            && self.material_mismatches == 0
            && self.nonfinite_distances == 0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParityReport {
    pub version: u32,
    pub adapter: String,
    pub backend: String,
    pub driver: String,
    pub driver_info: String,
    pub seed: u32,
    pub arithmetic: Vec<crate::det_math::MathParity>,
    pub graphs: Vec<GraphParity>,
}

impl ParityReport {
    /// A successful finite experiment is evidence for this device, driver,
    /// shader and corpus; it is not a proof over every point or every device.
    pub fn require_bit_exact(&self) -> Result<()> {
        let bundled = syfrack_core::graph::bundled::BUNDLED;
        ensure!(self.graphs.len() == bundled.len() && self.graphs.iter().zip(bundled)
            .all(|(g, (name, _))| g.graph == *name && g.points >= 100_000),
            "strict gate requires at least 100000 points for every bundled graph in canonical order");
        ensure!(
            self.arithmetic.len() == 8 && self.arithmetic.iter().all(|r| r.inputs >= 100_000),
            "strict gate requires at least 100000 inputs for each arithmetic operation"
        );
        ensure!(!self.arithmetic.is_empty() && self.arithmetic.iter().all(|r| r.mismatches == 0),
            "bit-exact arithmetic gate failed; shared polynomials and explicit WGSL fma are insufficient on this adapter");
        let failures: Vec<_> = self.graphs.iter().filter(|g| !g.is_bit_exact()).collect();
        ensure!(failures.is_empty(),
            "bit-exact parity gate failed for {}/{} graphs; native CPU/GPU interchange is not certified",
            failures.len(), self.graphs.len());
        Ok(())
    }
}

fn bits(s: &DeSample) -> [u32; 7] {
    [
        s.distance.to_bits(),
        s.traps[0].to_bits(),
        s.traps[1].to_bits(),
        s.traps[2].to_bits(),
        s.traps[3].to_bits(),
        s.iterations,
        s.material,
    ]
}

fn ordered(x: f32) -> u32 {
    let b = x.to_bits();
    if b & 0x8000_0000 != 0 {
        !b
    } else {
        b ^ 0x8000_0000
    }
}

fn compare(name: &str, points: &[Vec3], cpu: &[DeSample], gpu: &[DeSample]) -> Result<GraphParity> {
    ensure!(
        !points.is_empty() && points.len() == cpu.len() && cpu.len() == gpu.len(),
        "parity result count differs from input"
    );
    let mut r = GraphParity {
        graph: name.into(),
        points: points.len(),
        distance_mismatches: 0,
        trap_mismatches: [0; 4],
        iteration_mismatches: 0,
        material_mismatches: 0,
        nonfinite_distances: 0,
        max_distance_ulp: 0,
        max_distance_absolute_error: 0.0,
        first_difference: None,
    };
    for ((p, a), b) in points.iter().zip(cpu).zip(gpu) {
        let (a_bits, b_bits) = (bits(a), bits(b));
        r.distance_mismatches += usize::from(a_bits[0] != b_bits[0]);
        for k in 0..4 {
            r.trap_mismatches[k] += usize::from(a_bits[k + 1] != b_bits[k + 1]);
        }
        r.iteration_mismatches += usize::from(a.iterations != b.iterations);
        r.material_mismatches += usize::from(a.material != b.material);
        if a.distance.is_finite() && b.distance.is_finite() {
            r.max_distance_absolute_error = r
                .max_distance_absolute_error
                .max((a.distance - b.distance).abs());
            r.max_distance_ulp = r
                .max_distance_ulp
                .max(ordered(a.distance).abs_diff(ordered(b.distance)));
        } else {
            r.nonfinite_distances += 1;
        }
        if r.first_difference.is_none()
            && (a_bits != b_bits || !a.distance.is_finite() || !b.distance.is_finite())
        {
            r.first_difference = Some(Difference {
                point: p.to_array(),
                cpu: a_bits,
                gpu: b_bits,
            });
        }
    }
    Ok(r)
}

/// Reproducible random points plus origin, axes, poles and signed zero. Points
/// are generated once on the host and uploaded unchanged to both evaluators.
pub fn corpus(count: usize, extent: f32, seed: u32) -> Result<Vec<Vec3>> {
    ensure!(
        (16..=1_000_000).contains(&count),
        "parity point count must be 16..=1000000"
    );
    ensure!(extent.is_finite() && extent > 0.0, "invalid parity extent");
    let mut points = vec![
        Vec3::ZERO,
        Vec3::splat(-0.0),
        Vec3::X,
        -Vec3::X,
        Vec3::Y,
        -Vec3::Y,
        Vec3::Z,
        -Vec3::Z,
        Vec3::splat(1e-12),
        Vec3::splat(-1e-12),
    ];
    let mut s = seed;
    let mut next = || {
        s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (s >> 8) as f32 * (1.0 / 16_777_216.0) * (2.0 * extent) - extent
    };
    while points.len() < count {
        points.push(Vec3::new(next(), next(), next()));
    }
    Ok(points)
}

/// Audit every bundled graph, with progress after each complete graph. The
/// public production probe also uses this lean pipeline implementation.
pub fn audit(
    renderer: &Renderer,
    count: usize,
    seed: u32,
    progress: &mut dyn FnMut(&GraphParity),
) -> Result<ParityReport> {
    let info = &renderer.context().info;
    let mut report = ParityReport {
        version: 2,
        adapter: info.name.clone(),
        backend: format!("{:?}", info.backend),
        driver: info.driver.clone(),
        driver_info: info.driver_info.clone(),
        seed,
        arithmetic: crate::det_math::audit(renderer, count)?,
        graphs: vec![],
    };
    for (name, _) in syfrack_core::graph::bundled::BUNDLED {
        let g = Graph::bundled(name).unwrap();
        let points = corpus(count, if *name == "mandelbox" { 8.0 } else { 2.0 }, seed)?;
        let plan = PreparedGraph::new(&g)?;
        let cpu: Vec<_> = points.par_iter().map(|p| plan.evaluate(*p)).collect();
        let gpu = probe(renderer, &g, &points).with_context(|| format!("parity probe {name}"))?;
        let result = compare(name, &points, &cpu, &gpu)?;
        progress(&result);
        report.graphs.push(result);
    }
    Ok(report)
}

pub(crate) fn probe(renderer: &Renderer, graph: &Graph, points: &[Vec3]) -> Result<Vec<DeSample>> {
    probe_scene(
        renderer,
        &Scene {
            fractal: syfrack_core::Fractal::Graph(graph.clone()),
            ..Scene::example()
        },
        points,
    )
}
pub(crate) fn probe_scene(
    renderer: &Renderer,
    scene: &Scene,
    points: &[Vec3],
) -> Result<Vec<DeSample>> {
    let graph = scene.fractal.graph();
    graph.validate()?;
    if points.is_empty() {
        return Ok(vec![]);
    }
    let compiled = graph_wgsl::compile(graph)?;
    let device = &renderer.ctx.device;
    let max_points = (device.limits().max_storage_buffer_binding_size as usize / 16)
        .min(device.limits().max_compute_workgroups_per_dimension as usize * 64);
    ensure!(
        points.len() <= max_points,
        "distance probe exceeds device limits ({max_points} points)"
    );
    renderer.with_scopes(|| {
        let src = shader_source::de_probe(&compiled.wgsl);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("distance-only probe"),
            source: wgpu::ShaderSource::Wgsl(src.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("distance-only probe"),
            layout: Some(&renderer.layouts.probe),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let mut uniforms = Uniforms::from_scene(scene, 1, 1, 1, 1);
        let prepared = renderer.prepare_phenomena(scene)?;
        let mut parameter_values = compiled.layout.pack(graph);
        crate::phenomena::append_parameters(
            scene,
            prepared.as_deref(),
            &mut parameter_values,
            &mut uniforms,
        )?;
        let bytes = points.len() as u64 * 16;
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let uniform = buffer(
            "probe uniforms",
            std::mem::size_of::<Uniforms>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        renderer
            .ctx
            .queue
            .write_buffer(&uniform, 0, bytemuck::bytes_of(&uniforms));
        let params = renderer.graph_params_buffer(&parameter_values)?;
        let input = buffer(
            "probe points",
            bytes,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let values: Vec<_> = points.iter().map(|p| [p.x, p.y, p.z, 0.0]).collect();
        renderer
            .ctx
            .queue
            .write_buffer(&input, 0, bytemuck::cast_slice(&values));
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        let output = buffer("probe values", bytes, usage);
        let traps = buffer("probe traps", bytes, usage);
        let entries = [
            (0, uniform.as_entire_binding()),
            (1, input.as_entire_binding()),
            (2, output.as_entire_binding()),
            (3, traps.as_entire_binding()),
            (4, params.binding()),
            (16, params.overflow_binding()),
        ]
        .into_iter()
        .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource })
        .collect::<Vec<_>>();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("distance-only probe"),
            layout: &renderer.layouts.probe_layout,
            entries: &entries,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups((points.len() as u32).div_ceil(64), 1, 1);
        }
        renderer.ctx.queue.submit([encoder.finish()]);
        let raw = renderer.ctx.read_buffer(&output, bytes)?;
        let raw_traps = renderer.ctx.read_buffer(&traps, bytes)?;
        let rows: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&raw);
        let trap_rows: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&raw_traps);
        renderer.ctx.check()?;
        Ok(rows
            .into_iter()
            .zip(trap_rows)
            .map(|(r, traps)| DeSample {
                distance: r[0],
                traps,
                iterations: r[2] as u32,
                material: r[3] as u32,
            })
            .collect())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(distance: f32) -> DeSample {
        DeSample {
            distance,
            traps: [1.0; 4],
            iterations: 0,
            material: 0,
        }
    }
    #[test]
    fn release_gate_has_a_positive_control_and_requires_the_complete_corpus() {
        let mut report = ParityReport {
            version: 2,
            adapter: "synthetic".into(),
            backend: "test".into(),
            driver: String::new(),
            driver_info: String::new(),
            seed: 11,
            arithmetic: ["sin", "cos", "atan2", "log2", "exp2", "pow", "fma", "sqrt"]
                .into_iter()
                .map(|operation| crate::det_math::MathParity {
                    operation: operation.into(),
                    inputs: 100_000,
                    mismatches: 0,
                    first_input: None,
                    first_bits: None,
                })
                .collect(),
            graphs: syfrack_core::graph::bundled::BUNDLED
                .iter()
                .map(|(name, _)| {
                    let mut r =
                        compare(name, &[Vec3::ZERO], &[sample(1.0)], &[sample(1.0)]).unwrap();
                    r.points = 100_000;
                    r
                })
                .collect(),
        };
        report.require_bit_exact().unwrap();
        report.arithmetic[6].mismatches = 1;
        assert!(report.require_bit_exact().is_err());
        report.arithmetic[6].mismatches = 0;
        report.graphs[0].points = 99_999;
        assert!(report.require_bit_exact().is_err());
        report.graphs[0].points = 100_000;
        report.graphs.pop();
        assert!(report.require_bit_exact().is_err());
    }
    #[test]
    fn strict_gate_rejects_one_ulp_signed_zero_metadata_and_nonfinite() {
        let points = [Vec3::ZERO];
        let a = [sample(1.0)];
        let mut b = a;
        assert!(compare("test", &points, &a, &b).unwrap().is_bit_exact());
        b[0].distance = f32::from_bits(1.0f32.to_bits() + 1);
        let r = compare("test", &points, &a, &b).unwrap();
        assert_eq!(r.max_distance_ulp, 1);
        assert!(!r.is_bit_exact());
        assert!(!compare("test", &points, &[sample(0.0)], &[sample(-0.0)])
            .unwrap()
            .is_bit_exact());
        b = a;
        b[0].iterations = 1;
        assert!(!compare("test", &points, &a, &b).unwrap().is_bit_exact());
        assert!(
            !compare("test", &points, &[sample(f32::NAN)], &[sample(f32::NAN)])
                .unwrap()
                .is_bit_exact()
        );
        assert!(compare("test", &[], &[], &[]).is_err());
    }
    #[test]
    fn corpus_is_reproducible_bounded_and_keeps_poles() {
        let a = corpus(100_000, 2.0, 11).unwrap();
        assert_eq!(a, corpus(100_000, 2.0, 11).unwrap());
        assert!(a.contains(&Vec3::Z) && a.contains(&-Vec3::Z));
        assert!(a.iter().all(|p| p.abs().max_element() <= 2.0));
        assert_eq!(a[1].x.to_bits(), (-0.0f32).to_bits());
        assert!(corpus(0, 2.0, 11).is_err());
    }
}
