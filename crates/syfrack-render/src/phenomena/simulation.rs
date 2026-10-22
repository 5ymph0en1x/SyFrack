//! Canonical finite-domain snapshot solvers. No wall clock enters their state.
use super::{fields::Evaluator, grid::Grid};
use anyhow::{ensure, Result};
use glam::{Vec2, Vec3, Vec4};
use rayon::prelude::*;
use syfrack_core::scene::phenomena::*;
use syfrack_core::Scene;

#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub mass_before: f64,
    pub mass_after: f64,
    pub residual: f32,
    pub eigenvalue: f32,
    pub occupied: usize,
    pub substeps: u64,
}
struct Random(u64);
impl Random {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^= z >> 31;
        ((z >> 40) as u32) as f32 / 16777216.0
    }
}
fn mul(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x)
}
fn guide(p: &Phenomena, point: Vec3, kind: GuideKind) -> f32 {
    p.guides
        .iter()
        .filter(|g| g.kind == kind)
        .map(|g| g.weight(point))
        .sum()
}
pub(super) fn mass(grid: &Grid) -> f64 {
    grid.data
        .iter()
        .map(|v| (v.x * v.x + v.y * v.y) as f64)
        .sum()
}
pub(super) fn coefficients(
    scene: &Scene,
    p: &Phenomena,
    s: &Simulation,
    previous: &[Grid],
    grid: &Grid,
) -> Result<Vec<Vec4>> {
    let eval = Evaluator::new(scene, p, previous)?;
    (0..grid.data.len())
        .into_par_iter()
        .map(|i| {
            let point = grid.point(i);
            let sample = |id: Option<u32>, default: f32| -> Result<f32> {
                Ok(if let Some(id) = id {
                    eval.sample(id, point)?.x
                } else {
                    default
                })
            };
            Ok(Vec4::new(
                sample(s.potential, 0.0)?,
                sample(s.source, 0.0)? + guide(p, point, GuideKind::Source),
                sample(s.mask, 1.0)?,
                guide(p, point, GuideKind::Protect),
            ))
        })
        .collect()
}
pub(super) fn seed(scene: &Scene, p: &Phenomena, s: &Simulation, grid: &mut Grid, is_wave: bool) {
    let mut random = Random(s.seed ^ scene.seed);
    for i in 0..grid.data.len() {
        let point = grid.point(i);
        let mut psi = Vec2::ZERO;
        let mut amplitude = 0.0;
        for seed in &s.seeds {
            let local = point - seed.center;
            let shape = (-0.5 * local.length_squared() / (seed.radius * seed.radius)).exp()
                * seed.amplitude;
            let phase = seed.phase + seed.winding as f32 * local.y.atan2(local.x);
            let core = if seed.winding == 0 {
                1.0
            } else {
                (Vec2::new(local.x, local.y).length() / seed.radius)
                    .tanh()
                    .powi(seed.winding.abs())
            };
            psi += Vec2::new(phase.cos(), phase.sin()) * (shape * core);
            amplitude += shape;
        }
        amplitude += guide(p, point, GuideKind::Source);
        amplitude *= 1.0 - guide(p, point, GuideKind::Suppress).clamp(0.0, 1.0);
        grid.data[i] = if is_wave {
            Vec4::new(psi.x, psi.y, 0.0, 0.0)
        } else {
            let v = (amplitude * 0.5 + (random.next() - 0.5) * 0.002).clamp(0.0, 1.0);
            Vec4::new(1.0 - v, v, 0.0, 0.0)
        };
    }
}
fn rate(grid: &Grid, coefficients: &[Vec4], model: &SimulationModel) -> Vec<Vec4> {
    (0..grid.data.len())
        .into_par_iter()
        .map(|i| {
            let v = grid.data[i];
            let c = coefficients[i];
            if c.z <= 0.0 || c.w >= 0.5 {
                return Vec4::ZERO;
            }
            match *model {
                SimulationModel::GrayScott {
                    diffusion,
                    feed,
                    kill,
                } => {
                    let lap = grid.laplacian(i, false);
                    let uvv = v.x * v.y * v.y;
                    let f = (feed + c.y).clamp(0.0, 1.0);
                    let k = (kill + c.x).clamp(0.0, 1.0);
                    Vec4::new(
                        diffusion[0] * lap.x - uvv + f * (1.0 - v.x),
                        diffusion[1] * lap.y + uvv - (f + k) * v.y,
                        0.0,
                        0.0,
                    )
                }
                SimulationModel::GrossPitaevskii {
                    dispersion,
                    interaction,
                    damping,
                } => {
                    let lap = grid.laplacian(i, true);
                    let potential = c.x + interaction * (v.x * v.x + v.y * v.y);
                    let h = Vec2::new(
                        -dispersion * lap.x + potential * v.x,
                        -dispersion * lap.y + potential * v.y,
                    );
                    Vec4::new(h.y - damping * h.x + c.y, -h.x - damping * h.y, 0.0, 0.0)
                }
                _ => unreachable!(),
            }
        })
        .collect()
}
fn evolved(grid: &Grid, rate: &[Vec4], dt: f32) -> Grid {
    Grid {
        domain: grid.domain.clone(),
        data: grid
            .data
            .par_iter()
            .zip(rate)
            .map(|(v, r)| *v + *r * dt)
            .collect(),
    }
}
fn pde(
    scene: &Scene,
    p: &Phenomena,
    s: &Simulation,
    previous: &[Grid],
) -> Result<(Grid, Diagnostics)> {
    let wave = matches!(s.model, SimulationModel::GrossPitaevskii { .. });
    let mut grid = Grid::zero(&p.domain);
    seed(scene, p, s, &mut grid, wave);
    let coeff = coefficients(scene, p, s, previous, &grid)?;
    for (i, v) in grid.data.iter_mut().enumerate() {
        if wave && coeff[i].z <= 0.0 {
            *v = Vec4::ZERO;
        }
    }
    let mut diagnostic = Diagnostics {
        mass_before: mass(&grid),
        ..Default::default()
    };
    let h = grid.spacing();
    let inv_sum = (0..3)
        .filter(|&a| p.domain.resolution[a] > 1)
        .map(|a| 1.0 / (h[a] * h[a]))
        .sum::<f32>();
    for _ in 0..s.steps {
        let max_rho = grid
            .data
            .iter()
            .map(|v| v.x * v.x + v.y * v.y)
            .fold(0.0, f32::max);
        let bound = match s.model {
            SimulationModel::GrayScott { diffusion, .. } => {
                2.0 * diffusion[0].max(diffusion[1]) * inv_sum + 4.0
            }
            SimulationModel::GrossPitaevskii {
                dispersion,
                interaction,
                ..
            } => {
                4.0 * dispersion * inv_sum
                    + interaction.abs() * max_rho
                    + coeff.iter().map(|c| c.x.abs()).fold(0.0, f32::max)
            }
            _ => unreachable!(),
        };
        let subdivisions = (s.dt * bound / 0.2).ceil().max(1.0) as u32;
        ensure!(
            subdivisions <= 4096,
            "simulation {}: time step requires too many stable substeps",
            s.name
        );
        ensure!(
            (diagnostic.substeps + u64::from(subdivisions)) * grid.data.len() as u64
                <= 1_000_000_000,
            "simulation {} exceeds snapshot work budget; reduce resolution/time",
            s.name
        );
        let dt = s.dt / subdivisions as f32;
        for _ in 0..subdivisions {
            let a = rate(&grid, &coeff, &s.model);
            let b = rate(&evolved(&grid, &a, dt * 0.5), &coeff, &s.model);
            let c = rate(&evolved(&grid, &b, dt * 0.5), &coeff, &s.model);
            let d = rate(&evolved(&grid, &c, dt), &coeff, &s.model);
            grid.data
                .par_iter_mut()
                .enumerate()
                .for_each(|(i, v)| *v += (a[i] + 2.0 * b[i] + 2.0 * c[i] + d[i]) * (dt / 6.0));
            diagnostic.substeps += 1;
        }
        ensure!(
            grid.data.iter().all(|v| v.is_finite()),
            "simulation {} diverged; change coefficients or initial conditions",
            s.name
        );
    }
    diagnostic.mass_after = mass(&grid);
    if wave {
        for v in &mut grid.data {
            v.z = v.x * v.x + v.y * v.y;
            v.w = v.y.atan2(v.x) / std::f32::consts::TAU + 0.5;
        }
    }
    Ok((grid, diagnostic))
}
fn grow(
    scene: &Scene,
    p: &Phenomena,
    s: &Simulation,
    previous: &[Grid],
    eta: f32,
    relaxations: u32,
    branches: u32,
) -> Result<(Grid, Diagnostics)> {
    let mut grid = Grid::zero(&p.domain);
    let coeff = coefficients(scene, p, s, previous, &grid)?;
    let mut occupied = vec![false; grid.data.len()];
    let mut blocked = vec![false; grid.data.len()];
    let mut random = Random(s.seed ^ scene.seed);
    for i in 0..grid.data.len() {
        let point = grid.point(i);
        occupied[i] = s.seeds.iter().any(|b| point.distance(b.center) <= b.radius)
            || guide(p, point, GuideKind::Source) > 0.5;
        blocked[i] = coeff[i].z <= 0.0
            || guide(p, point, GuideKind::Suppress) > 0.5
            || guide(p, point, GuideKind::Protect) > 0.5;
        grid.data[i].y = if occupied[i] { 0.0 } else { 1.0 };
    }
    ensure!(
        occupied.iter().any(|v| *v),
        "growth seeds miss the grid; increase seed radius"
    );
    let h = grid.spacing();
    let weights = Vec3::ONE / (h * h);
    let denominator = 2.0
        * (0..3)
            .filter(|&a| p.domain.resolution[a] > 1)
            .map(|a| weights[a])
            .sum::<f32>();
    ensure!(
        u64::from(s.steps) * u64::from(relaxations) * grid.data.len() as u64 <= 1_000_000_000,
        "growth snapshot exceeds relaxation work budget"
    );
    for _ in 0..s.steps {
        for _ in 0..relaxations {
            let next: Vec<_> = (0..grid.data.len())
                .into_par_iter()
                .map(|i| {
                    if occupied[i] {
                        return 0.0;
                    }
                    let c = grid.coord(i);
                    if (0..3).any(|a| {
                        p.domain.resolution[a] > 1
                            && (c[a] == 0 || c[a] + 1 == p.domain.resolution[a])
                    }) {
                        return 1.0;
                    }
                    if blocked[i] {
                        return grid.data[i].y;
                    }
                    let mut sum = 0.0;
                    for a in 0..3 {
                        if p.domain.resolution[a] > 1 {
                            for d in [-1, 1] {
                                let j = grid.neighbor(i, a, d, false);
                                sum += weights[a]
                                    * if blocked[j] {
                                        grid.data[i].y
                                    } else {
                                        grid.data[j].y
                                    };
                            }
                        }
                    }
                    sum / denominator
                })
                .collect();
            for (v, n) in grid.data.iter_mut().zip(next) {
                v.y = n;
            }
        }
        let mut candidates = vec![];
        let mut total = 0.0f64;
        for i in 0..grid.data.len() {
            if occupied[i] || blocked[i] {
                continue;
            }
            let mut touches = false;
            let mut grad = Vec3::ZERO;
            for a in 0..3 {
                if p.domain.resolution[a] > 1 {
                    touches |= occupied[grid.neighbor(i, a, -1, false)]
                        || occupied[grid.neighbor(i, a, 1, false)];
                    grad[a] = (grid.data[grid.neighbor(i, a, 1, false)].y
                        - grid.data[grid.neighbor(i, a, -1, false)].y)
                        / (2.0 * h[a]);
                }
            }
            if touches {
                let bias =
                    1.0 + guide(p, grid.point(i), GuideKind::Attractor) + coeff[i].y.max(0.0);
                // Exponentiation in f64 prevents high dielectric exponents
                // from silently discarding the strongest frontier candidates.
                let weight = (grad.length().max(1e-8) as f64).powf(eta as f64) * bias as f64;
                if weight.is_finite() && weight > 0.0 {
                    total += weight;
                    candidates.push((i, weight));
                }
            }
        }
        for _ in 0..branches.min(candidates.len() as u32) {
            if total <= 0.0 {
                break;
            }
            let mut target = random.next() as f64 * total;
            let mut chosen = candidates.len() - 1;
            for (j, (_, weight)) in candidates.iter().enumerate() {
                target -= weight;
                if target <= 0.0 {
                    chosen = j;
                    break;
                }
            }
            let (index, weight) = candidates.swap_remove(chosen);
            occupied[index] = true;
            grid.data[index].y = 0.0;
            total -= weight;
        }
    }
    for (i, v) in grid.data.iter_mut().enumerate() {
        v.x = f32::from(occupied[i]);
    }
    let diagnostic = Diagnostics {
        occupied: occupied.iter().filter(|v| **v).count(),
        ..Default::default()
    };
    Ok((grid, diagnostic))
}
fn schottky(
    scene: &Scene,
    p: &Phenomena,
    s: &Simulation,
    pairs: &[CirclePair],
    points: u32,
    burn_in: u32,
) -> Result<(Grid, Diagnostics)> {
    let mut grid = Grid::zero(&p.domain);
    let mut random = Random(s.seed ^ scene.seed);
    let mut z = Vec2::splat(
        pairs
            .iter()
            .map(|p| {
                p.source[0]
                    .abs()
                    .max(p.source[1].abs())
                    .max(p.target[0].abs())
                    .max(p.target[1].abs())
                    + p.radius
            })
            .fold(1.0, f32::max)
            * 3.0,
    );
    let count = pairs.len() * 2;
    let mut last = count;
    for sample in 0..points.saturating_add(burn_in) {
        let mut branch =
            (random.next() * (if last == count { count } else { count - 1 }) as f32) as usize;
        if last != count && branch >= (last ^ 1) {
            branch += 1;
        }
        branch = branch.min(count - 1);
        let pair = &pairs[branch / 2];
        let (source, target) = if branch % 2 == 0 {
            (pair.source, pair.target)
        } else {
            (pair.target, pair.source)
        };
        let den = z - Vec2::from_array(source);
        ensure!(
            den.length_squared() > 1e-20,
            "Schottky orbit encountered a pole"
        );
        let angle = pair.twist.to_radians();
        let scale = Vec2::new(angle.cos(), angle.sin()) * (pair.radius * pair.radius);
        z = Vec2::from_array(target) + mul(scale, Vec2::new(den.x, -den.y)) / den.length_squared();
        last = branch;
        if sample >= burn_in {
            let point = Vec3::new(z.x, z.y, p.domain.center.z);
            if !grid.contains(point) {
                continue;
            }
            let uv = (point - p.domain.center + p.domain.half) / (p.domain.half * 2.0);
            let r = p.domain.resolution;
            let x = (uv.x * (r[0] - 1) as f32).round() as u32;
            let y = (uv.y * (r[1] - 1) as f32).round() as u32;
            let zi = (r[2] - 1) / 2;
            let index = grid.index(x.min(r[0] - 1), y.min(r[1] - 1), zi);
            grid.data[index].x += 1.0;
        }
    }
    let maximum = grid.data.iter().map(|v| v.x).fold(0.0, f32::max);
    let scale = (1.0 + maximum).ln().max(1.0);
    for v in &mut grid.data {
        v.x = (1.0 + v.x).ln() / scale;
    }
    Ok((grid, Diagnostics::default()))
}
fn resonance(
    scene: &Scene,
    p: &Phenomena,
    s: &Simulation,
    previous: &[Grid],
    mode: u32,
    iterations: u32,
) -> Result<(Grid, Diagnostics)> {
    let mut grid = Grid::zero(&p.domain);
    let coeff = coefficients(scene, p, s, previous, &grid)?;
    let h = grid.spacing();
    let sum = (0..3)
        .filter(|&a| p.domain.resolution[a] > 1)
        .map(|a| 1.0 / (h[a] * h[a]))
        .sum::<f32>();
    let tau = 0.2 / sum;
    let mask: Vec<_> = (0..grid.data.len())
        .map(|i| {
            let c = grid.coord(i);
            coeff[i].z > 0.0
                && !(0..3).any(|a| {
                    p.domain.resolution[a] > 1 && (c[a] == 0 || c[a] + 1 == p.domain.resolution[a])
                })
        })
        .collect();
    ensure!(
        mask.iter().filter(|v| **v).count() > mode as usize + 2,
        "resonance domain has too few interior cells"
    );
    let mut modes: Vec<Vec<f32>> = vec![];
    let mut rng = Random(s.seed ^ scene.seed);
    ensure!(
        u64::from(mode + 1) * u64::from(iterations) * grid.data.len() as u64 <= 500_000_000,
        "eigenmode work budget exceeded"
    );
    for _ in 0..=mode {
        let mut values: Vec<f32> = mask
            .iter()
            .map(|active| if *active { rng.next() - 0.5 } else { 0.0 })
            .collect();
        for _ in 0..iterations {
            for (i, v) in grid.data.iter_mut().enumerate() {
                v.x = values[i];
            }
            let mut next: Vec<f32> = (0..values.len())
                .into_par_iter()
                .map(|i| {
                    if mask[i] {
                        values[i] + tau * grid.laplacian(i, false).x
                    } else {
                        0.0
                    }
                })
                .collect();
            for previous in &modes {
                let projection = next
                    .iter()
                    .zip(previous)
                    .map(|(a, b)| (*a as f64) * (*b as f64))
                    .sum::<f64>() as f32;
                for (a, b) in next.iter_mut().zip(previous) {
                    *a -= projection * *b;
                }
            }
            let norm = next
                .iter()
                .map(|x| (*x as f64) * (*x as f64))
                .sum::<f64>()
                .sqrt() as f32;
            ensure!(norm > 1e-20, "degenerate eigenmode iteration");
            for v in &mut next {
                *v /= norm;
            }
            values = next;
        }
        modes.push(values);
    }
    let values = modes.last().unwrap();
    for (i, v) in grid.data.iter_mut().enumerate() {
        v.x = values[i];
    }
    let av: Vec<f32> = (0..values.len())
        .map(|i| {
            if mask[i] {
                -grid.laplacian(i, false).x
            } else {
                0.0
            }
        })
        .collect();
    let eigenvalue: f32 = av.iter().zip(values).map(|(a, b)| a * b).sum();
    let residual = av
        .iter()
        .zip(values)
        .map(|(a, b)| (a - eigenvalue * b).powi(2))
        .sum::<f32>()
        .sqrt()
        / eigenvalue.abs().max(1e-10);
    ensure!(
        residual < 0.1,
        "eigenmode did not converge (relative residual {residual:.4}); increase solver iterations"
    );
    let max = values
        .iter()
        .map(|v| v.abs())
        .fold(0.0, f32::max)
        .max(1e-20);
    for v in &mut grid.data {
        v.x /= max;
    }
    Ok((
        grid,
        Diagnostics {
            residual,
            eigenvalue,
            ..Default::default()
        },
    ))
}
pub fn prepare(
    scene: &Scene,
    p: &Phenomena,
    index: usize,
    previous: &[Grid],
) -> Result<(Grid, Diagnostics)> {
    let s = &p.simulations[index];
    if let Some(frozen) = &s.frozen {
        return Ok((
            Grid {
                domain: p.domain.clone(),
                data: frozen.values.iter().map(|v| Vec4::from_array(*v)).collect(),
            },
            Diagnostics::default(),
        ));
    }
    match &s.model {
        SimulationModel::GrayScott { .. } | SimulationModel::GrossPitaevskii { .. } => {
            pde(scene, p, s, previous)
        }
        SimulationModel::DielectricGrowth {
            eta,
            relaxation_steps,
            branches_per_step,
        } => grow(
            scene,
            p,
            s,
            previous,
            *eta,
            *relaxation_steps,
            *branches_per_step,
        ),
        SimulationModel::Schottky {
            pairs,
            points,
            burn_in,
        } => schottky(scene, p, s, pairs, *points, *burn_in),
        SimulationModel::Resonance { mode, iterations } => {
            resonance(scene, p, s, previous, *mode, *iterations)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup(model: SimulationModel) -> (Scene, Phenomena) {
        let mut p = Phenomena::default();
        p.domain.resolution = [17, 17, 1];
        p.domain.half = Vec3::ONE;
        p.simulations = vec![Simulation {
            model,
            steps: 10,
            dt: 0.01,
            ..Default::default()
        }];
        let mut s = Scene::example();
        s.phenomena = Some(p.clone());
        (s, p)
    }
    #[test]
    fn nonlinear_wave_preserves_mass_without_damping() {
        let (s, p) = setup(SimulationModel::GrossPitaevskii {
            dispersion: 0.05,
            interaction: 0.2,
            damping: 0.0,
        });
        let (a, da) = prepare(&s, &p, 0, &[]).unwrap();
        let (b, db) = prepare(&s, &p, 0, &[]).unwrap();
        assert_eq!(a.data, b.data);
        assert_eq!(da.mass_after, db.mass_after);
        assert!((da.mass_after / da.mass_before - 1.0).abs() < 1e-4);
        assert!(a
            .data
            .iter()
            .all(|v| (v.z - v.x * v.x - v.y * v.y).abs() < 1e-6));
    }
    #[test]
    fn reaction_diffusion_is_finite_reproducible_and_nontrivial() {
        let (s, p) = setup(SimulationModel::GrayScott {
            diffusion: [0.16, 0.08],
            feed: 0.035,
            kill: 0.062,
        });
        let (a, _) = prepare(&s, &p, 0, &[]).unwrap();
        let (b, _) = prepare(&s, &p, 0, &[]).unwrap();
        assert_eq!(a.data, b.data);
        assert!(a.data.iter().all(|v| v.is_finite()));
        assert!(a.data.iter().any(|v| v.y > 0.1));
    }
    #[test]
    fn dielectric_growth_increases_connected_occupied_set() {
        let (s, p) = setup(SimulationModel::DielectricGrowth {
            eta: 2.0,
            relaxation_steps: 16,
            branches_per_step: 2,
        });
        let mut initial = p.clone();
        initial.simulations[0].steps = 0;
        let (_, before) = prepare(&s, &initial, 0, &[]).unwrap();
        let (a, after) = prepare(&s, &p, 0, &[]).unwrap();
        assert!(after.occupied > before.occupied);
        let (b, _) = prepare(&s, &p, 0, &[]).unwrap();
        assert_eq!(a.data, b.data);
    }
    #[test]
    fn dirichlet_box_fundamental_has_expected_discrete_eigenvalue() {
        let (s, mut p) = setup(SimulationModel::Resonance {
            mode: 0,
            iterations: 1024,
        });
        p.simulations[0].steps = 0;
        let (g, d) = prepare(&s, &p, 0, &[]).unwrap();
        let h = g.spacing().x;
        let expected = 8.0 / (h * h) * (std::f32::consts::PI / 32.0).sin().powi(2);
        assert!((d.eigenvalue / expected - 1.0).abs() < 0.01);
        assert!(d.residual < 0.01);
    }
}
