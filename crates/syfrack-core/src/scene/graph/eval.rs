use super::{Graph, Measure, NodeId, NodeKind, TrapMode, MAX_DEPTH};
use crate::scene::fractal::DeSample;
use glam::{EulerRot, Quat, Vec2, Vec3};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct PointState {
    pub p: Vec3,
    pub dr: f32,
    pub iter: u32,
    pub traps: [f32; 4],
}
#[derive(Clone, Copy, Debug)]
pub struct DistState {
    pub d: f32,
    pub material: u32,
    pub iter: u32,
    pub traps: [f32; 4],
}

/// CPU evaluation plan and instance BVHs prepared once for an immutable graph.
/// Share this object between CPU workers instead of rebuilding plans per pixel.
pub struct PreparedGraph<'a> {
    graph: &'a Graph,
    output: NodeId,
    plans: HashMap<NodeId, Vec<NodeId>>,
    instances: HashMap<NodeId, Vec<super::bvh::InstanceBounds>>,
    trap_scales: [f32; 4],
    iteration_product: u32,
}

impl<'a> PreparedGraph<'a> {
    pub fn new(g: &'a Graph) -> Result<Self, crate::SceneError> {
        let order = g.topo_order()?;
        fn needed(g: &Graph, id: NodeId, set: &mut std::collections::HashSet<NodeId>) {
            if !set.insert(id) {
                return;
            }
            let Some(n) = g.node(id) else {
                return;
            };
            if n.enabled && matches!(n.kind, NodeKind::Instances { .. }) {
                return;
            }
            for port in 0..n.kind.inputs().len() as u32 {
                if let Some((src, _)) = g.input_of(id, port) {
                    needed(g, src, set);
                }
            }
        }
        let output = g
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Output)
            .map(|n| n.id)
            .unwrap_or(0);
        let mut targets = vec![output];
        for node in &g.nodes {
            if node.enabled && matches!(node.kind, NodeKind::Instances { .. }) {
                if let Some((child, _)) = g.input_of(node.id, 0) {
                    targets.push(child);
                }
            }
        }
        let mut plans = HashMap::new();
        for target in targets {
            let mut set = std::collections::HashSet::new();
            needed(g, target, &mut set);
            plans.insert(
                target,
                order
                    .iter()
                    .copied()
                    .filter(|id| set.contains(id))
                    .collect::<Vec<_>>(),
            );
        }
        let instances = g
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Instances {
                    placements, half, ..
                } if n.enabled => Some((n.id, super::bvh::build(placements, *half))),
                _ => None,
            })
            .collect();
        Ok(Self {
            graph: g,
            output,
            plans,
            instances,
            trap_scales: g.trap_scales(),
            iteration_product: g.iteration_product(),
        })
    }

    pub fn evaluate(&self, p: Vec3) -> DeSample {
        let r = self.graph.evaluate_raw(self.output, p, self, 0);
        let scales = self.trap_scales;
        let traps = std::array::from_fn(|i| {
            if r.traps[i].is_finite() {
                (r.traps[i] / scales[i]).min(1.0)
            } else {
                1.0
            }
        });
        DeSample {
            distance: r.d,
            material: r.material,
            traps,
            iterations: r.iter.min(self.iteration_product),
        }
    }
}

enum Value {
    Point(PointState),
    Dist(DistState),
}

fn rot2(v: Vec2, a: f32) -> Vec2 {
    let (s, c) = a.sin_cos();
    Vec2::new(c * v.x - s * v.y, s * v.x + c * v.y)
}

/// Integer spherical powers avoid inverse trigonometry. Using rho/r for the
/// polar sine also preserves thin features where z/r rounds to exactly +/-1.
fn integral_spherical_power(p: Vec3, r: f32, n: u32) -> (Vec3, f32) {
    if n == 1 {
        return (p, 1.0);
    }
    let mul = |a: Vec2, b: Vec2| Vec2::new(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
    let rho = Vec2::new(p.x, p.y).length();
    let mut theta = Vec2::new((p.z / r).clamp(-1.0, 1.0), rho / r);
    let mut phi = if rho > 0.0 {
        Vec2::new(p.x, p.y) / rho
    } else {
        Vec2::X
    };
    if n == 6 || n == 8 {
        let square = |a: Vec2| Vec2::new(a.x * a.x - a.y * a.y, 2.0 * a.x * a.y);
        let (t2, f2) = (square(theta), square(phi));
        let (t4, f4) = (square(t2), square(f2));
        let r2 = r * r;
        let r4 = r2 * r2;
        let (t, f, previous) = if n == 6 {
            (mul(t2, t4), mul(f2, f4), r4 * r)
        } else {
            (square(t4), square(f4), r4 * r2 * r)
        };
        return (
            Vec3::new(t.y * f.x, t.y * f.y, t.x) * (previous * r),
            n as f32 * previous,
        );
    }
    let (mut t, mut f) = (Vec2::X, Vec2::X);
    let mut exponent = n;
    while exponent != 0 {
        if exponent & 1 != 0 {
            t = mul(t, theta);
            f = mul(f, phi);
        }
        exponent >>= 1;
        if exponent != 0 {
            theta = mul(theta, theta);
            phi = mul(phi, phi);
        }
    }
    let mut radial_previous = 1.0;
    for _ in 1..n {
        radial_previous *= r;
    }
    (
        Vec3::new(t.y * f.x, t.y * f.y, t.x) * (radial_previous * r),
        n as f32 * radial_previous,
    )
}

#[test]
fn integer_spherical_powers_match_double_precision_and_preserve_poles() {
    let mut seed = 12341u32;
    let mut points = vec![
        Vec3::X,
        Vec3::Y,
        Vec3::Z,
        -Vec3::Z,
        Vec3::new(1e-6, 0.0, 1.0),
        Vec3::new(-1e-6, 2e-6, -1.0),
    ];
    for _ in 0..1000 {
        let mut next = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / 16777216.0 * 3.0 - 1.5
        };
        points.push(Vec3::new(next(), next(), next()));
    }
    for n in 1..=16 {
        for &p in &points {
            let q = p.as_dvec3();
            let r = q.length();
            let theta = (q.z / r).clamp(-1.0, 1.0).acos() * n as f64;
            let phi = q.y.atan2(q.x) * n as f64;
            let expected = glam::DVec3::new(
                theta.sin() * phi.cos(),
                theta.sin() * phi.sin(),
                theta.cos(),
            ) * r.powi(n as i32);
            let (actual, derivative) = integral_spherical_power(p, p.length(), n);
            assert!(
                (actual.as_dvec3() - expected).length() < 5e-6 * r.powi(n as i32).max(1e-20),
                "n={n}, p={p}"
            );
            let expected_dr = n as f64 * r.powi(n as i32 - 1);
            assert!((derivative as f64 - expected_dr).abs() < 4e-6 * expected_dr.max(1e-20));
        }
    }
    let p = Vec3::new(1e-6, 0.0, 1.0);
    assert_eq!(p.z / p.length(), 1.0); // acos of this rounded cosine loses the filament.
    let (actual, _) = integral_spherical_power(p, p.length(), 8);
    assert!((actual.x - 8e-6).abs() < 1e-10);
}

/// An empty distance: nothing on this branch (a disabled fence is +inf; the
/// GPU uses 1e30, hence the threshold).
fn is_empty(d: f32) -> bool {
    d >= 1e30
}

fn terrain_hash(cell: [i32; 3], seed: u32) -> f32 {
    let mut h = (cell[0] as u32).wrapping_mul(0x8da6b343)
        ^ (cell[1] as u32).wrapping_mul(0xd8163841)
        ^ (cell[2] as u32).wrapping_mul(0xcb1ab31f)
        ^ seed;
    h = (h ^ (h >> 16)).wrapping_mul(0x7feb352d);
    h = (h ^ (h >> 15)).wrapping_mul(0x846ca68b);
    h ^= h >> 16;
    (h >> 8) as f32 / 16777216.0
}
fn terrain_noise(p: Vec3, seed: u32) -> f32 {
    let base = p.floor();
    let f = p - base;
    let t = f * f * (Vec3::splat(3.0) - 2.0 * f);
    let c = base.as_ivec3().to_array();
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let hash = |x, y, z| terrain_hash([c[0] + x, c[1] + y, c[2] + z], seed);
    let a = mix(hash(0, 0, 0), hash(1, 0, 0), t.x);
    let b = mix(hash(0, 1, 0), hash(1, 1, 0), t.x);
    let d = mix(hash(0, 0, 1), hash(1, 0, 1), t.x);
    let e = mix(hash(0, 1, 1), hash(1, 1, 1), t.x);
    mix(mix(a, b, t.y), mix(d, e, t.y), t.z)
}
fn rock_distance(p: Vec3, half: Vec3, erosion: f32, seed: u32) -> f32 {
    let h = half.min_element();
    let q = p * (1.15 / h);
    let relief = 0.57 * terrain_noise(q, seed)
        + 0.29 * terrain_noise(q * 2.03, seed + 97)
        + 0.14 * terrain_noise(q * 4.17, seed + 193)
        - 0.5;
    ((p / half).length() - 1.0 - erosion * relief) * h / (1.0 + 5.22 * erosion)
}

#[cfg(test)]
#[test]
fn eroded_rock_is_bounded_and_safe_for_sphere_tracing() {
    let half = Vec3::new(4.0, 1.2, 2.7);
    for erosion in [0.0, 0.3, 0.6] {
        for seed in [0, 317, 65535] {
            for i in 0..2500 {
                let p = Vec3::new(
                    (i % 17) as f32 * 0.71 - 5.7,
                    ((i / 17) % 17) as f32 * 0.23 - 1.8,
                    (i / 289) as f32 * 0.63 - 2.8,
                );
                let d = rock_distance(p, half, erosion, seed);
                assert!(d.is_finite());
                let eps = 0.001;
                let gradient = Vec3::new(
                    rock_distance(p + Vec3::X * eps, half, erosion, seed)
                        - rock_distance(p - Vec3::X * eps, half, erosion, seed),
                    rock_distance(p + Vec3::Y * eps, half, erosion, seed)
                        - rock_distance(p - Vec3::Y * eps, half, erosion, seed),
                    rock_distance(p + Vec3::Z * eps, half, erosion, seed)
                        - rock_distance(p - Vec3::Z * eps, half, erosion, seed),
                ) / (2.0 * eps);
                assert!(gradient.length() <= 1.002, "{p}: {gradient}");
                if (p / half).length() > 1.0 + erosion * 0.5 {
                    assert!(d > 0.0);
                }
            }
        }
    }
    assert_ne!(
        rock_distance(Vec3::X, half, 0.4, 317),
        rock_distance(Vec3::X, half, 0.4, 318)
    );
}

fn measure(m: Measure, p: Vec3) -> f32 {
    match m {
        Measure::Norm => p.length(),
        // Distance to the axis (the length of the two other components).
        Measure::AxisX => Vec2::new(p.y, p.z).length(),
        Measure::AxisY => Vec2::new(p.x, p.z).length(),
        Measure::AxisZ => Vec2::new(p.x, p.y).length(),
        // Distance to the plane y = 0.
        Measure::PlaneY => p.y.abs(),
        Measure::AngleXY => {
            if p.x == 0.0 && p.y == 0.0 {
                0.5
            } else {
                p.y.atan2(p.x) / std::f32::consts::TAU + 0.5
            }
        }
        Measure::AngleXZ => {
            if p.x == 0.0 && p.z == 0.0 {
                0.5
            } else {
                p.z.atan2(p.x) / std::f32::consts::TAU + 0.5
            }
        }
        Measure::AngleYZ => {
            if p.y == 0.0 && p.z == 0.0 {
                0.5
            } else {
                p.z.atan2(p.y) / std::f32::consts::TAU + 0.5
            }
        }
        Measure::LogNorm => p.length().ln_1p(),
    }
}

impl Graph {
    pub fn body_ids_of(&self, iterate: NodeId) -> &[NodeId] {
        match self.node(iterate).map(|n| &n.kind) {
            Some(NodeKind::Iterate { body, .. }) => body,
            _ => &[],
        }
    }

    /// Applies one Point → Point node. `c` is the Input point (for AddOrigin).
    fn apply(&self, id: NodeId, s: PointState, c: Vec3) -> PointState {
        self.apply_seeded(id, s, c, (s.p, s.dr))
    }

    fn apply_seeded(
        &self,
        id: NodeId,
        mut s: PointState,
        c: Vec3,
        seed: (Vec3, f32),
    ) -> PointState {
        let Some(node) = self.node(id) else { return s };
        if !node.enabled {
            return s;
        }
        use NodeKind::*;
        match &node.kind {
            Translate { offset } => s.p += *offset,
            Rotate { angles } => {
                let q = Quat::from_euler(
                    EulerRot::XYZ,
                    angles.x.to_radians(),
                    angles.y.to_radians(),
                    angles.z.to_radians(),
                );
                s.p = q * s.p;
            }
            Scale { factor } => {
                s.p *= *factor;
                s.dr *= factor.abs();
            }
            ScaleAxes { factors } => {
                s.p *= *factors;
                s.dr *= factors.max_element();
            }
            Mirror { normal, offset } => {
                let n = normal.try_normalize().unwrap_or(Vec3::Y);
                let h = s.p.dot(n) - offset;
                if h < 0.0 {
                    s.p -= 2.0 * h * n;
                }
            }
            BoxFold { limit } => {
                s.p = s.p.clamp(Vec3::splat(-*limit), Vec3::splat(*limit)) * 2.0 - s.p
            }
            BoxFoldAxes { limit, x, y, z } => {
                let folded = s.p.clamp(-*limit, *limit) * 2.0 - s.p;
                for (axis, enabled) in [*x, *y, *z].into_iter().enumerate() {
                    if enabled {
                        s.p[axis] = folded[axis];
                    }
                }
            }
            SphereInversion { radius, min_radius } => {
                let k = radius * radius / s.p.length_squared().max(min_radius * min_radius);
                s.p *= k;
                // The inversion Jacobian is k times a reflection; its norm is k.
                // The regularised core is linear with the same bound.
                s.dr *= k;
            }
            SphereFold {
                min_radius,
                fixed_radius,
            } => {
                let r2 = s.p.dot(s.p);
                let mr2 = min_radius * min_radius;
                let fr2 = fixed_radius * fixed_radius;
                let k = if r2 < mr2 {
                    fr2 / mr2
                } else if r2 < fr2 {
                    fr2 / r2
                } else {
                    1.0
                };
                s.p *= k;
                s.dr *= k;
            }
            Abs { x, y, z } => {
                if *x {
                    s.p.x = s.p.x.abs();
                }
                if *y {
                    s.p.y = s.p.y.abs();
                }
                if *z {
                    s.p.z = s.p.z.abs();
                }
            }
            MengerFold { scale, offset } => {
                let mut z = s.p.abs();
                if z.x < z.y {
                    z = Vec3::new(z.y, z.x, z.z);
                }
                if z.x < z.z {
                    z = Vec3::new(z.z, z.y, z.x);
                }
                if z.y < z.z {
                    z = Vec3::new(z.x, z.z, z.y);
                }
                z = z * *scale - *offset * (scale - 1.0);
                if z.z < -0.5 * offset.z * (scale - 1.0) {
                    z.z += offset.z * (scale - 1.0);
                }
                s.p = z;
                s.dr *= *scale;
            }
            Kaleido { planes, phase } => {
                let sector = std::f32::consts::TAU / (*planes).max(1) as f32;
                let a = s.p.z.atan2(s.p.x) + phase.to_radians();
                let a = ((a + sector * 0.5).rem_euclid(sector) - sector * 0.5).abs();
                let r = Vec2::new(s.p.x, s.p.z).length();
                s.p.x = r * a.cos();
                s.p.z = r * a.sin();
            }
            Power { power } => {
                let radius = s.p.length();
                if radius >= 1e-12 && (1.0..=16.0).contains(power) && power.fract() == 0.0 {
                    let (point, derivative) = integral_spherical_power(s.p, radius, *power as u32);
                    s.p = point;
                    s.dr *= derivative;
                } else {
                    let r = radius.max(1e-12);
                    let theta = if radius >= 1e-12 {
                        Vec2::new(s.p.x, s.p.y).length().atan2(s.p.z)
                    } else {
                        (s.p.z / r).clamp(-1.0, 1.0).acos()
                    };
                    let phi = s.p.y.atan2(s.p.x);
                    s.dr *= r.powf(power - 1.0) * power;
                    let zr = r.powf(*power);
                    let (st, ct) = (theta * power).sin_cos();
                    let (sp, cp) = (phi * power).sin_cos();
                    s.p = Vec3::new(st * cp, st * sp, ct) * zr;
                }
            }
            AddOrigin => {
                s.p += c;
                s.dr += 1.0;
            }
            AddSeed { multiplier } => {
                s.p += seed.0 * *multiplier;
                s.dr += seed.1 * multiplier.abs().max_element();
            }
            AddConstant { multiplier, julia } => {
                s.p += c * *multiplier + *julia;
                // Norm of the diagonal input multiplier. Julia is constant in c.
                s.dr += multiplier.abs().max_element();
            }
            Twist { rate } => {
                let v = rot2(Vec2::new(s.p.x, s.p.z), rate * s.p.y);
                s.p.x = v.x;
                s.p.z = v.y;
            }
            Bend { rate } => {
                let v = rot2(Vec2::new(s.p.x, s.p.y), rate * s.p.x);
                s.p.x = v.x;
                s.p.y = v.y;
            }
            Tile { period } => {
                let half = *period * 0.5;
                s.p = Vec3::new(
                    (s.p.x + half.x).rem_euclid(period.x) - half.x,
                    (s.p.y + half.y).rem_euclid(period.y) - half.y,
                    (s.p.z + half.z).rem_euclid(period.z) - half.z,
                );
            }
            RepeatFinite { period, count } => {
                let last = Vec3::new(count[0] as f32, count[1] as f32, count[2] as f32) - Vec3::ONE;
                let half = last * 0.5;
                let cell = (s.p / *period + half + Vec3::splat(0.5))
                    .floor()
                    .clamp(Vec3::ZERO, last)
                    - half;
                s.p -= cell * *period;
            }
            Iterate {
                iterations,
                bailout,
                body,
            } => {
                let local_seed = (s.p, s.dr);
                for _ in 0..*iterations {
                    if s.p.length() > *bailout {
                        break;
                    }
                    for b in body {
                        s = self.apply_seeded(*b, s, c, local_seed);
                    }
                    s.iter += 1;
                }
            }
            Trap {
                slot,
                measure: m,
                mode,
                ..
            } => {
                let i = (*slot as usize).min(3);
                let v = measure(*m, s.p);
                s.traps[i] = match mode {
                    TrapMode::Minimum => s.traps[i].min(v),
                    TrapMode::Maximum => {
                        if s.traps[i] >= 1e30 {
                            v
                        } else {
                            s.traps[i].max(v)
                        }
                    }
                    TrapMode::Last => v,
                };
            }
            _ => {}
        }
        s
    }

    fn fence(kind: &NodeKind, s: PointState) -> f32 {
        use NodeKind::*;
        match kind {
            Sphere { radius } => (s.p.length() - radius) / s.dr,
            Torus { radius, tube } => {
                (Vec2::new(Vec2::new(s.p.x, s.p.z).length() - radius, s.p.y).length() - tube) / s.dr
            }
            Cylinder {
                radius,
                half_height,
            } => {
                let q = Vec2::new(
                    Vec2::new(s.p.x, s.p.z).length() - radius,
                    s.p.y.abs() - half_height,
                );
                (q.max(Vec2::ZERO).length() + q.max_element().min(0.0)) / s.dr
            }
            Box { half } => {
                let q = s.p.abs() - *half;
                (q.max(Vec3::ZERO).length() + q.x.max(q.y.max(q.z)).min(0.0)) / s.dr
            }
            RoundBox { half, radius } => {
                let r = radius.min(half.min_element());
                let q = s.p.abs() - (*half - Vec3::splat(r));
                (q.max(Vec3::ZERO).length() + q.max_element().min(0.0) - r) / s.dr
            }
            Rock {
                half,
                erosion,
                seed,
            } => rock_distance(s.p, *half, *erosion, *seed) / s.dr,
            Plane { normal, offset } => {
                (s.p.dot(normal.try_normalize().unwrap_or(Vec3::Y)) - offset) / s.dr
            }
            NormFence { inset } => (s.p.length() - inset) / s.dr.abs(),
            BulbFence => {
                let r = s.p.length().max(1e-12);
                0.5 * r.ln() * r / s.dr
            }
            _ => f32::INFINITY,
        }
    }

    fn combine(kind: &NodeKind, a: DistState, b: DistState) -> DistState {
        use NodeKind::*;
        let pick = |take_a: bool| if take_a { a } else { b };
        let blend = |h: f32, d: f32| {
            // An empty slot (+inf) on one side must not poison the other.
            let traps =
                std::array::from_fn(|i| match (a.traps[i].is_finite(), b.traps[i].is_finite()) {
                    (true, true) => b.traps[i] + (a.traps[i] - b.traps[i]) * h,
                    (true, false) => a.traps[i],
                    (false, true) => b.traps[i],
                    (false, false) => f32::INFINITY,
                });
            DistState {
                d,
                material: if h >= 0.5 { a.material } else { b.material },
                iter: if h >= 0.5 { a.iter } else { b.iter },
                traps,
            }
        };
        match kind {
            Union => {
                let mut r = pick(a.d <= b.d);
                r.d = a.d.min(b.d);
                r
            }
            Intersection => {
                let mut r = pick(a.d >= b.d);
                r.d = a.d.max(b.d);
                r
            }
            Subtraction => DistState {
                d: a.d.max(-b.d),
                ..a
            },
            // An empty operand (+inf: a disabled fence) is settled before any
            // blending, which would otherwise produce NaN (the WGSL twin does the same).
            SmoothUnion { .. } if is_empty(a.d) => b,
            SmoothUnion { .. } if is_empty(b.d) => a,
            SmoothIntersection { .. } if is_empty(a.d) => DistState {
                d: f32::INFINITY,
                ..b
            },
            SmoothIntersection { .. } if is_empty(b.d) => DistState {
                d: f32::INFINITY,
                ..a
            },
            SmoothUnion { k } => {
                let h = (0.5 + 0.5 * (b.d - a.d) / k).clamp(0.0, 1.0);
                blend(h, b.d + (a.d - b.d) * h - k * h * (1.0 - h))
            }
            SmoothIntersection { k } => {
                let h = (0.5 - 0.5 * (b.d - a.d) / k).clamp(0.0, 1.0);
                blend(h, b.d + (a.d - b.d) * h + k * h * (1.0 - h))
            }
            _ => a,
        }
    }

    pub fn evaluate(&self, p: Vec3) -> DeSample {
        match PreparedGraph::new(self) {
            Ok(prepared) => prepared.evaluate(p),
            Err(_) => DeSample {
                distance: p.length(),
                material: 0,
                traps: [1.0; 4],
                iterations: 0,
            },
        }
    }

    fn evaluate_raw(
        &self,
        target: NodeId,
        p: Vec3,
        prepared: &PreparedGraph<'_>,
        depth: u32,
    ) -> DistState {
        let empty = DistState {
            d: f32::INFINITY,
            material: 0,
            iter: 0,
            traps: [f32::INFINITY; 4],
        };
        if depth > MAX_DEPTH {
            return empty;
        }
        let Some(order) = prepared.plans.get(&target) else {
            return empty;
        };
        let start = PointState {
            p,
            dr: 1.0,
            iter: 0,
            traps: [f32::INFINITY; 4],
        };
        let mut values: HashMap<NodeId, Value> = HashMap::new();
        for &id in order {
            let node = self.node(id).unwrap();
            let get_point = |port: u32, values: &HashMap<NodeId, Value>| match self
                .input_of(id, port)
                .and_then(|(from, _)| values.get(&from))
            {
                Some(Value::Point(s)) => *s,
                _ => start,
            };
            let get_dist = |port: u32, values: &HashMap<NodeId, Value>| match self
                .input_of(id, port)
                .and_then(|(from, _)| values.get(&from))
            {
                Some(Value::Dist(d)) => *d,
                _ => empty,
            };
            let v = match &node.kind {
                NodeKind::Input => Value::Point(start),
                NodeKind::Output => Value::Dist(get_dist(0, &values)),
                k if k.is_point_to_point() => {
                    Value::Point(self.apply(id, get_point(0, &values), p))
                }
                k if k.is_fence() => {
                    let s = get_point(0, &values);
                    Value::Dist(DistState {
                        d: if node.enabled {
                            Self::fence(k, s)
                        } else {
                            f32::INFINITY
                        },
                        material: 0,
                        iter: s.iter,
                        traps: s.traps,
                    })
                }
                NodeKind::MaterialTag { index } => {
                    let mut d = get_dist(0, &values);
                    if node.enabled {
                        d.material = *index;
                    }
                    Value::Dist(d)
                }
                NodeKind::Bounded { half, .. } => {
                    let mut d = get_dist(1, &values);
                    if node.enabled {
                        d.d = d.d.max(Self::fence(
                            &NodeKind::Box { half: *half },
                            get_point(0, &values),
                        ));
                    }
                    Value::Dist(d)
                }
                NodeKind::Instances {
                    placements, half, ..
                } if node.enabled => {
                    let Some((child, _)) = self.input_of(id, 0) else {
                        values.insert(id, Value::Dist(empty));
                        continue;
                    };
                    let nodes = &prepared.instances[&id];
                    let mut best = empty;
                    let mut chosen = usize::MAX;
                    let mut seed = 0;
                    if !nodes.is_empty() {
                        while nodes[seed].placement.is_none() {
                            let left = seed + 1;
                            let right = nodes[left].escape;
                            seed = if nodes[left].distance(p) <= nodes[right].distance(p) {
                                left
                            } else {
                                right
                            };
                        }
                    }
                    let mut cursor = seed;
                    let mut seed_pass = true;
                    while cursor < nodes.len() {
                        let current = cursor;
                        let first = seed_pass;
                        seed_pass = false;
                        cursor = if first { 0 } else { current + 1 };
                        let node = &nodes[current];
                        if node.distance(p) > best.d {
                            if !first {
                                cursor = node.escape;
                            }
                            continue;
                        }
                        if !first && current == seed {
                            continue;
                        }
                        let Some(index) = node.placement else {
                            continue;
                        };
                        let placement = &placements[index];
                        let a = placement.angles * (std::f32::consts::PI / 180.0);
                        let rotation = Quat::from_euler(EulerRot::XYZ, a.x, a.y, a.z);
                        let q = (rotation.conjugate() * (p - placement.position)) / placement.scale;
                        let scale = placement.scale.min_element();
                        let bound = Self::fence(
                            &NodeKind::Box {
                                half: *half * placement.scale,
                            },
                            PointState {
                                p: q * placement.scale,
                                ..start
                            },
                        );
                        if bound > best.d {
                            continue;
                        }
                        let mut d = self.evaluate_raw(child, q, prepared, depth + 1);
                        d.d = (d.d * scale).max(bound);
                        if d.d < best.d || (d.d == best.d && index < chosen) {
                            best = d;
                            chosen = index;
                        }
                    }
                    Value::Dist(best)
                }
                _ if !node.enabled => Value::Dist(get_dist(0, &values)),
                k => Value::Dist(Self::combine(k, get_dist(0, &values), get_dist(1, &values))),
            };
            values.insert(id, v);
        }
        match values.get(&target) {
            Some(Value::Dist(d)) => *d,
            _ => empty,
        }
    }

    pub fn lipschitz(&self, radius: f32) -> f32 {
        fn visit(g: &Graph, id: NodeId, radius: f32, depth: u32, l: &mut f32) {
            if depth > MAX_DEPTH + 1 {
                return;
            }
            let Some(n) = g.node(id) else { return };
            if !n.enabled {
                return;
            }
            if let NodeKind::Twist { rate } | NodeKind::Bend { rate } = n.kind {
                *l *= (1.0 + (rate * radius).powi(2)).sqrt();
            }
            for b in g.body_ids_of(id) {
                visit(g, *b, radius, depth + 1, l);
            }
        }
        let mut l = 1.0f32;
        for id in self.active_nodes() {
            visit(self, id, radius, 0, &mut l);
        }
        l
    }

    /// Normalisation scale of each trap slot: the last enabled Trap writing
    /// it in topological order (bodies in body order), whether or not it runs.
    pub fn trap_scales(&self) -> [f32; 4] {
        fn walk(g: &Graph, id: NodeId, depth: u32, scales: &mut [f32; 4]) {
            if depth > MAX_DEPTH + 1 {
                return;
            }
            let Some(n) = g.node(id) else { return };
            if !n.enabled {
                return;
            }
            match &n.kind {
                NodeKind::Trap { slot, scale, .. } => scales[(*slot as usize).min(3)] = *scale,
                NodeKind::Iterate { body, .. } => {
                    for b in body {
                        walk(g, *b, depth + 1, scales);
                    }
                }
                _ => {}
            }
        }
        let mut scales = [1.0f32; 4];
        for id in self.topo_order().unwrap_or_default() {
            walk(self, id, 0, &mut scales);
        }
        scales
    }

    pub fn step_scale(&self) -> f32 {
        self.safety / self.lipschitz(self.bounding_radius_hint)
    }

    pub fn estimate_bounding_radius(&self) -> f32 {
        let Ok(prepared) = PreparedGraph::new(self) else {
            return self.bounding_radius_hint;
        };
        let mut dirs = Vec::new();
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    if (x, y, z) != (0, 0, 0) {
                        dirs.push(Vec3::new(x as f32, y as f32, z as f32).normalize());
                    }
                }
            }
        }
        let mut radius = 0.5f32;
        let mut bounded = false;
        for d in dirs {
            let mut r = 100.0f32;
            // Already at the surface at the far end: the form is unbounded
            // that way (a ground plane), the direction says nothing of its extent.
            let far = prepared.evaluate(d * r).distance;
            if !far.is_finite() || far <= 0.05 * r {
                continue;
            }
            bounded = true;
            // Walk inward while the field stays positive and decreasing outward.
            while r > 0.5 {
                let dist = prepared.evaluate(d * r).distance;
                if !dist.is_finite() || dist <= 0.05 * r {
                    break;
                }
                r -= dist.min(r * 0.5).max(0.01);
            }
            radius = radius.max(r);
        }
        if !bounded {
            return 100.0;
        }
        radius.clamp(0.5, 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::graph::Node;

    fn transformed(kind: NodeKind, p: Vec3) -> PointState {
        let mut g = Graph::new_minimal();
        g.nodes.push(Node {
            id: 3,
            kind,
            name: "test".into(),
            pos: [0.0; 2],
            enabled: true,
        });
        g.apply(
            3,
            PointState {
                p,
                dr: 1.0,
                iter: 0,
                traps: [f32::INFINITY; 4],
            },
            p,
        )
    }

    #[test]
    fn hybrid_axis_fold_preserves_unselected_axes_and_matches_legacy() {
        for p in [Vec3::new(-2.4, 0.3, 3.2), Vec3::ZERO, Vec3::splat(0.7)] {
            let old = transformed(NodeKind::BoxFold { limit: 1.0 }, p);
            let all = transformed(
                NodeKind::BoxFoldAxes {
                    limit: Vec3::ONE,
                    x: true,
                    y: true,
                    z: true,
                },
                p,
            );
            assert_eq!(old.p, all.p);
            assert_eq!(all.dr, 1.0);
            let planar = transformed(
                NodeKind::BoxFoldAxes {
                    limit: Vec3::ONE,
                    x: true,
                    y: true,
                    z: false,
                },
                p,
            );
            assert_eq!(planar.p.z, p.z);
            assert_eq!([planar.p.x, planar.p.y], [old.p.x, old.p.y]);
        }
    }

    #[test]
    fn hybrid_inversion_is_involutive_and_its_derivative_bounds_local_stretch() {
        let kind = NodeKind::SphereInversion {
            radius: 1.2,
            min_radius: 0.1,
        };
        for p in [Vec3::new(1.3, -0.3, 0.2), Vec3::new(0.4, 0.6, -0.2)] {
            let a = transformed(kind.clone(), p);
            let b = transformed(kind.clone(), a.p);
            assert!((p - b.p).length() < 1e-5);
            assert!((a.dr * b.dr - 1.0).abs() < 1e-5);
        }
        // Include the regularised core and the origin: no NaN, no division by zero.
        for p in [Vec3::ZERO, Vec3::splat(0.01), Vec3::new(0.3, -0.5, 0.8)] {
            let a = transformed(kind.clone(), p);
            assert!(a.p.is_finite() && a.dr.is_finite());
            for direction in [Vec3::X, Vec3::Y, Vec3::Z, Vec3::ONE.normalize()] {
                let h = 1e-4;
                let plus = transformed(kind.clone(), p + direction * h).p;
                let minus = transformed(kind.clone(), p - direction * h).p;
                assert!((plus - minus).length() / (2.0 * h) <= a.dr * 1.002);
            }
        }
    }

    #[test]
    fn hybrid_constant_keeps_legacy_and_julia_derivatives_distinct() {
        let p = Vec3::new(0.4, -1.2, 2.3);
        let legacy = transformed(NodeKind::AddOrigin, p);
        let seeded = transformed(
            NodeKind::AddConstant {
                multiplier: Vec3::ONE,
                julia: Vec3::ZERO,
            },
            p,
        );
        assert_eq!(legacy.p, seeded.p);
        assert_eq!(legacy.dr, seeded.dr);
        let julia = transformed(
            NodeKind::AddConstant {
                multiplier: Vec3::ZERO,
                julia: Vec3::new(-0.2, 0.4, 0.1),
            },
            p,
        );
        assert_eq!(julia.dr, 1.0);
        let mixed = transformed(
            NodeKind::AddConstant {
                multiplier: Vec3::new(-2.0, 0.4, 0.0),
                julia: Vec3::ZERO,
            },
            p,
        );
        assert_eq!(mixed.dr, 3.0);
        assert_eq!(mixed.p.z, p.z);
    }

    #[test]
    fn axis_measures_are_distances_to_the_axes() {
        let p = Vec3::new(3.0, -4.0, 12.0);
        assert_eq!(measure(Measure::AxisX, p), Vec2::new(-4.0, 12.0).length());
        assert_eq!(measure(Measure::AxisY, p), Vec2::new(3.0, 12.0).length());
        assert_eq!(measure(Measure::AxisZ, p), 5.0);
        assert_eq!(measure(Measure::PlaneY, p), 4.0);
        assert_eq!(measure(Measure::Norm, p), 13.0);
    }

    fn rotate(angles: Vec3, v: Vec3) -> Vec3 {
        let mut g = Graph::new_minimal();
        g.nodes.push(Node {
            id: 3,
            kind: NodeKind::Rotate { angles },
            name: "r".into(),
            pos: [0.0; 2],
            enabled: true,
        });
        let s = PointState {
            p: v,
            dr: 1.0,
            iter: 0,
            traps: [f32::INFINITY; 4],
        };
        g.apply(3, s, v).p
    }

    fn close(a: Vec3, b: Vec3) {
        assert!((a - b).length() < 1e-6, "{a} vs {b}");
    }

    #[test]
    fn rotate_follows_the_glam_intrinsic_xyz_convention() {
        // Single axes: right-handed, counter-clockwise seen from the axis tip.
        close(rotate(Vec3::new(90.0, 0.0, 0.0), Vec3::Y), Vec3::Z);
        close(rotate(Vec3::new(0.0, 90.0, 0.0), Vec3::Z), Vec3::X);
        close(rotate(Vec3::new(0.0, 0.0, 90.0), Vec3::X), Vec3::Y);
        // Combined (90, 90, 0) = Rx * Ry * v: Ry is applied first, then Rx.
        close(rotate(Vec3::new(90.0, 90.0, 0.0), Vec3::X), Vec3::Y);
        close(rotate(Vec3::new(90.0, 90.0, 0.0), Vec3::Z), Vec3::X);
        close(rotate(Vec3::new(90.0, 90.0, 0.0), Vec3::Y), Vec3::Z);
    }

    /// Sphere (radius 1, port 0) and a disabled Box (port 1) into `kind`.
    fn sphere_and_disabled_box(kind: NodeKind) -> Graph {
        use crate::scene::graph::Link;
        let mut g = Graph::new_minimal();
        let node = |id, kind: NodeKind, enabled| Node {
            id,
            name: kind.label().into(),
            kind,
            pos: [0.0; 2],
            enabled,
        };
        g.nodes.push(node(
            3,
            NodeKind::Trap {
                slot: 1,
                measure: Measure::Norm,
                scale: 4.0,
                mode: Default::default(),
            },
            true,
        ));
        g.nodes
            .push(node(4, NodeKind::Box { half: Vec3::ONE }, false));
        g.nodes.push(node(5, kind, true));
        let l = |a, b, port| Link {
            from: (a, 0),
            to: (b, port),
        };
        g.links = vec![
            l(0, 3, 0),
            l(3, 1, 0),
            l(0, 4, 0),
            l(1, 5, 0),
            l(4, 5, 1),
            l(5, 2, 0),
        ];
        g.validate().unwrap();
        g
    }

    #[test]
    fn a_smooth_union_with_an_empty_side_returns_the_other_side() {
        let g = sphere_and_disabled_box(NodeKind::SmoothUnion { k: 0.3 });
        let p = Vec3::new(2.0, 0.5, 0.0);
        let s = g.evaluate(p);
        assert_eq!(s.distance, p.length() - 1.0);
        assert_eq!(s.traps, [1.0, p.length() / 4.0, 1.0, 1.0]);
        // Port 0 empty instead: same answer.
        let mut g = g;
        g.links.retain(|l| l.to.0 != 5);
        g.links.push(crate::scene::graph::Link {
            from: (4, 0),
            to: (5, 0),
        });
        g.links.push(crate::scene::graph::Link {
            from: (1, 0),
            to: (5, 1),
        });
        assert_eq!(g.evaluate(p).distance, p.length() - 1.0);
    }

    #[test]
    fn a_smooth_intersection_with_an_empty_side_is_empty() {
        let g = sphere_and_disabled_box(NodeKind::SmoothIntersection { k: 0.3 });
        let s = g.evaluate(Vec3::new(2.0, 0.5, 0.0));
        assert!(
            s.distance.is_infinite() && s.distance > 0.0,
            "{}",
            s.distance
        );
        assert!(s.traps.iter().all(|t| t.is_finite()));
        assert_eq!(s.traps[1], Vec3::new(2.0, 0.5, 0.0).length() / 4.0);
    }
}
