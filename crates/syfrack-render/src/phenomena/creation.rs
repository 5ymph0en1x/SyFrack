//! A fixed GPU field machine. Editing connections uploads a typed program;
//! analytic pattern detail is evaluated at each actual image/path sample.
use super::{grid::Grid, PreparedPhenomena};
use crate::{
    uniforms::{GpuPlanar, GpuRamp, GpuTexture},
    Uniforms,
};
use anyhow::{ensure, Result};
use std::collections::{BTreeMap, BTreeSet};
use syfrack_core::{scene::phenomena::*, Scene};

fn append(values: &mut Vec<f32>, rows: &[[f32; 4]]) -> u32 {
    let start = (values.len() / 4) as u32;
    for row in rows {
        values.extend_from_slice(row);
    }
    start
}
fn grid(values: &mut Vec<f32>, g: &Grid) -> [f32; 4] {
    let start = (values.len() / 4) as f32;
    for v in &g.data {
        values.extend_from_slice(&v.to_array());
    }
    [
        start,
        g.domain.resolution[0] as f32,
        g.domain.resolution[1] as f32,
        g.domain.resolution[2] as f32,
    ]
}
fn pod_rows<T: bytemuck::Pod>(v: &T) -> Vec<[f32; 4]> {
    bytemuck::pod_collect_to_vec(bytemuck::bytes_of(v))
}
pub fn append_program(
    scene: &Scene,
    prepared: &PreparedPhenomena,
    values: &mut Vec<f32>,
    u: &mut Uniforms,
) -> Result<()> {
    if scene.creation.is_none() {
        return Ok(());
    }
    let p = scene.phenomena.as_ref().unwrap();
    let roots = [
        p.bindings.color,
        p.bindings.emission,
        p.bindings.density,
        None,
        None,
        p.bindings.film_thickness,
        p.bindings.rgb,
    ];
    let mut active = BTreeSet::new();
    let mut pending: Vec<_> = roots.into_iter().flatten().collect();
    while let Some(id) = pending.pop() {
        if active.insert(id) && !prepared.fallbacks.contains_key(&id) {
            pending.extend(&p.fields.nodes.iter().find(|n| n.id == id).unwrap().inputs);
        }
    }
    let order: Vec<_> = p
        .fields
        .order()?
        .into_iter()
        .filter(|id| active.contains(id))
        .collect();
    if order.is_empty() {
        return Ok(());
    }
    ensure!(
        order.len() <= MAX_FIELD_NODES,
        "creation runtime register budget exceeded"
    );
    let registers: BTreeMap<_, _> = order
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i as f32))
        .collect();
    let commands = (values.len() / 4) as u32;
    values.resize(values.len() + order.len() * 8, 0.0);
    let mut snapshots = BTreeMap::new();
    let mut flames = BTreeMap::new();
    for (index, id) in order.iter().enumerate() {
        let n = p.fields.nodes.iter().find(|n| n.id == *id).unwrap();
        let mut inputs = [-1.0; 3];
        for (j, id) in n.inputs.iter().enumerate() {
            if let Some(reg) = registers.get(id) {
                inputs[j] = *reg;
            }
        }
        let (code, data) = if let Some(fallback) = prepared.fallbacks.get(id) {
            (50, vec![grid(values, fallback)])
        } else {
            match &n.op {
                FieldOp::Position => (0, vec![]),
                FieldOp::Time => (1, vec![[p.time, 0.0, 0.0, 0.0]]),
                FieldOp::Scalar { value } => (2, vec![[*value, 0.0, 0.0, 0.0]]),
                FieldOp::Vector { value } => (2, vec![[value.x, value.y, value.z, 0.0]]),
                FieldOp::Complex { value } => (2, vec![[value[0], value[1], 0.0, 0.0]]),
                FieldOp::Add => (3, vec![]),
                FieldOp::Subtract => (4, vec![]),
                FieldOp::Multiply => (5, vec![]),
                FieldOp::Scale => (6, vec![]),
                FieldOp::Dot => (7, vec![]),
                FieldOp::Cross => (8, vec![]),
                FieldOp::Length => (9, vec![]),
                FieldOp::Normalize => (10, vec![]),
                FieldOp::Sine => (11, vec![]),
                FieldOp::Cosine => (12, vec![]),
                FieldOp::Exponential => (13, vec![]),
                FieldOp::Absolute => (14, vec![]),
                FieldOp::Clamp { min, max } => (15, vec![[*min, *max, 0.0, 0.0]]),
                FieldOp::Power { exponent } => (16, vec![[*exponent, 0.0, 0.0, 0.0]]),
                FieldOp::ComplexProduct => (17, vec![]),
                FieldOp::ComplexPower { power } => (18, vec![[*power as f32, 0.0, 0.0, 0.0]]),
                FieldOp::Phase => (19, vec![]),
                FieldOp::Intensity => (20, vec![]),
                FieldOp::PlaneWave {
                    wavevector,
                    frequency,
                    phase,
                } => (
                    21,
                    vec![
                        [wavevector.x, wavevector.y, wavevector.z, *frequency],
                        [*phase, p.time, 0.0, 0.0],
                    ],
                ),
                FieldOp::GaussianWave {
                    center,
                    width,
                    wavevector,
                    frequency,
                } => (
                    22,
                    vec![
                        [center.x, center.y, center.z, *width],
                        [wavevector.x, wavevector.y, wavevector.z, *frequency],
                        [p.time, 0.0, 0.0, 0.0],
                    ],
                ),
                FieldOp::Snapshot {
                    simulation,
                    channel,
                } => {
                    let metadata = if let Some(m) = snapshots.get(simulation) {
                        *m
                    } else {
                        let m = grid(values, &prepared.snapshots[*simulation as usize]);
                        snapshots.insert(*simulation, m);
                        m
                    };
                    (23, vec![metadata, [*channel as f32, 0.0, 0.0, 0.0]])
                }
                FieldOp::Planar { fractal, channels } => {
                    let gpu = GpuPlanar::from_planar(Some(fractal));
                    let mut rows = pod_rows(&gpu);
                    rows[0] = gpu.kind.map(|v| v as f32);
                    rows.push(channels.traps);
                    rows.push([
                        channels.iterations,
                        channels.normal,
                        channels.logarithmic,
                        0.0,
                    ]);
                    (24, rows)
                }
                FieldOp::Texture { texture } => {
                    let gpu = GpuTexture::from_texture(Some(texture));
                    let mut rows = pod_rows(&gpu);
                    rows[0] = gpu.kind.map(|v| v as f32);
                    rows[4] = gpu.ramp.kind.map(|v| v as f32);
                    rows[27] = gpu.planar.kind.map(|v| v as f32);
                    (32, rows)
                }
                FieldOp::Flame { source, channel } => {
                    let metadata = if let Some(m) = flames.get(source) {
                        *m
                    } else {
                        let f = &prepared.flames[*source as usize];
                        let offset = (values.len() / 4) as f32;
                        for v in f.values.iter() {
                            values.extend_from_slice(v);
                        }
                        let m = [offset, f.resolution[0] as f32, 0.0, 0.0];
                        flames.insert(*source, m);
                        m
                    };
                    let f = &p.flames[*source as usize];
                    let (sn, cs) = f.angle.to_radians().sin_cos();
                    (
                        25,
                        vec![
                            metadata,
                            [f.center[0], f.center[1], f.half[0], f.half[1]],
                            [
                                sn,
                                cs,
                                if *channel == FlameChannel::Color {
                                    0.0
                                } else {
                                    1.0
                                },
                                0.0,
                            ],
                        ],
                    )
                }
                FieldOp::Palette { ramp } => {
                    let gpu = GpuRamp::from_ramp(ramp);
                    let mut rows = pod_rows(&gpu);
                    rows[0] = gpu.kind.map(|v| v as f32);
                    (26, rows)
                }
                FieldOp::Mix => (27, vec![]),
                FieldOp::Luminance => (28, vec![]),
                FieldOp::Mobius { a, b, c, d } => {
                    (29, vec![[a[0], a[1], b[0], b[1]], [c[0], c[1], d[0], d[1]]])
                }
                FieldOp::ComplexFromParts => (30, vec![]),
                FieldOp::SphereDistance { center, radius } => {
                    (31, vec![[center.x, center.y, center.z, *radius]])
                }
                _ => unreachable!("finite operators must have prepared grids"),
            }
        };
        let at = append(values, &data);
        let command = (commands as usize + index * 2) * 4;
        values[command..command + 8].copy_from_slice(&[
            code as f32,
            inputs[0],
            inputs[1],
            inputs[2],
            at as f32,
            0.0,
            0.0,
            0.0,
        ]);
    }
    let root_offset = (values.len() / 4) as u32;
    let mut indices = [-1.0; 8];
    for (i, root) in roots.into_iter().enumerate() {
        if let Some(id) = root {
            indices[i] = registers[&id];
        }
    }
    values.extend_from_slice(&indices);
    u.phenomena.program = [commands, order.len() as u32, root_offset, 0];
    ensure!(
        values.len() / 4 < 16_777_216,
        "creation parameter texture exceeds exact address range"
    );
    Ok(())
}
