//! Complex-plane orbit fields for native material textures; never used as solid geometry.
use crate::params::{join, visit_u32, Bounds, Params};
use crate::DeSample;
use glam::Vec2;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanarFamily {
    #[default]
    Julia,
    Mandelbrot,
    BurningShip,
    Newton,
    Nova,
    Phoenix,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlanarFractal {
    pub family: PlanarFamily,
    pub power: u32,
    pub iterations: u32,
    pub julia: [f32; 2],
    /// Translation in the complex plane, after the texture's usual world mapping.
    pub center: [f32; 2],
    pub zoom: f32,
    pub angle: f32,
    /// Bounded spherical inversion of the domain. Zero leaves it unchanged.
    pub inversion: f32,
    /// Rotation of each polynomial step, or imaginary part of Newton relaxation.
    pub twist: f32,
    pub relaxation: f32,
    /// Real coefficient of the previous iterate in the Phoenix recurrence.
    pub memory: f32,
    pub bailout: f32,
    pub trap_radius: f32,
    pub stripes: f32,
}

impl Default for PlanarFractal {
    fn default() -> Self {
        Self {
            family: PlanarFamily::Julia,
            power: 2,
            iterations: 128,
            julia: [-0.74543, 0.11301],
            center: [0.0, 0.0],
            zoom: 1.0,
            angle: 0.0,
            inversion: 0.0,
            twist: 0.0,
            relaxation: 1.0,
            memory: -0.5,
            bailout: 16.0,
            trap_radius: 0.5,
            stripes: 12.0,
        }
    }
}

pub fn complex_mul(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x)
}
pub fn complex_power(z: Vec2, power: u32) -> Vec2 {
    let mut result = Vec2::X;
    for _ in 0..power {
        result = complex_mul(result, z);
    }
    result
}
fn complex_div(a: Vec2, b: Vec2) -> Vec2 {
    complex_mul(a, Vec2::new(b.x, -b.y)) / b.length_squared().max(1e-24)
}

impl PlanarFractal {
    pub fn sample(&self, input: Vec2) -> DeSample {
        let a = self.angle.to_radians();
        let (sn, cs) = a.sin_cos();
        let mut point =
            Vec2::new(cs * input.x - sn * input.y, sn * input.x + cs * input.y) / self.zoom;
        if self.inversion != 0.0 {
            point *= self.inversion / point.length_squared().max(1e-8);
        }
        point += Vec2::from_array(self.center);
        let parameter_plane = matches!(
            self.family,
            PlanarFamily::Mandelbrot | PlanarFamily::BurningShip
        );
        let root = matches!(self.family, PlanarFamily::Newton | PlanarFamily::Nova);
        let c = if parameter_plane {
            point
        } else {
            Vec2::from_array(self.julia)
        };
        let mut z = if parameter_plane { Vec2::ZERO } else { point };
        let mut previous = Vec2::ZERO;
        let rotation = Vec2::new(self.twist.to_radians().cos(), self.twist.to_radians().sin());
        let lambda = rotation * self.relaxation;
        let mut ring = 1e20f32;
        let mut stripe = 0.0;
        let mut previous_error = 1.0f32;
        let mut count = 0u32;
        let mut measured = 0u32;
        let mut smooth = self.iterations as f32;
        let mut escaped = false;
        for _ in 0..self.iterations {
            let radius = z.length();
            if radius > self.bailout {
                escaped = true;
                break;
            }
            ring = ring.min((radius - self.trap_radius).abs());
            let angle = if radius == 0.0 { 0.0 } else { z.y.atan2(z.x) };
            stripe += 0.5 + 0.5 * (self.stripes * angle).sin();
            measured += 1;
            if root {
                let residual = complex_power(z, self.power) - Vec2::X;
                if self.family == PlanarFamily::Newton && residual.length_squared() < 1e-12 {
                    let error = residual.length().max(1e-20);
                    let fraction = ((1e-6 / previous_error).ln() / (error / previous_error).ln())
                        .clamp(0.0, 1.0);
                    smooth = if count == 0 {
                        0.0
                    } else {
                        count as f32 - 1.0 + fraction
                    };
                    break;
                }
                previous_error = residual.length().max(1e-20);
                let old = z;
                let derivative = complex_power(z, self.power - 1) * self.power as f32;
                z -= complex_mul(lambda, complex_div(residual, derivative));
                if self.family == PlanarFamily::Nova {
                    z += c;
                    if (z - old).length_squared() < 1e-12 {
                        count += 1;
                        smooth = count as f32;
                        break;
                    }
                }
            } else {
                let base = if self.family == PlanarFamily::BurningShip {
                    z.abs()
                } else {
                    z
                };
                let next = complex_mul(rotation, complex_power(base, self.power))
                    + c
                    + if self.family == PlanarFamily::Phoenix {
                        previous * self.memory
                    } else {
                        Vec2::ZERO
                    };
                previous = z;
                z = next;
            }
            count += 1;
        }
        let radius = z.length().min(1e20);
        escaped |= radius > self.bailout;
        if escaped && !root {
            smooth = count as f32 + 1.0 - radius.max(1.0001).ln().ln() / (self.power as f32).ln();
        } else if escaped {
            smooth = count as f32;
        }
        let final_angle = if radius == 0.0 {
            0.5
        } else {
            z.y.atan2(z.x) / std::f32::consts::TAU + 0.5
        };
        let mut bands = stripe / measured.max(1) as f32;
        if escaped && !root && measured > 0 {
            let angle = if radius == 0.0 { 0.0 } else { z.y.atan2(z.x) };
            let last = 0.5 + 0.5 * (self.stripes * angle).sin();
            let full = (stripe + last) / (measured + 1) as f32;
            let t = (1.0
                + (self.bailout.ln().ln() - radius.max(1.0001).ln().ln())
                    / (self.power as f32).ln())
            .clamp(0.0, 1.0);
            bands += (full - bands) * t;
        }
        DeSample {
            material: 0,
            distance: radius / (1.0 + radius),
            iterations: count,
            traps: [
                (smooth / self.iterations as f32).clamp(0.0, 1.0),
                ring / (1.0 + ring),
                bands,
                final_angle,
            ],
        }
    }
}

impl Params for PlanarFractal {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        visit_u32(
            &join(prefix, "power"),
            &mut self.power,
            Bounds::new(2.0, 8.0, &[(2.0, 4.0)]),
            f,
        );
        visit_u32(
            &join(prefix, "iterations"),
            &mut self.iterations,
            Bounds::new(4.0, 512.0, &[(64.0, 256.0)]),
            f,
        );
        for (field, v) in [("julia", &mut self.julia), ("center", &mut self.center)] {
            for (i, x) in v.iter_mut().enumerate() {
                f(
                    &format!("{prefix}.{field}[{i}]"),
                    x,
                    Bounds::new(-16.0, 16.0, &[(-2.0, 2.0)]),
                );
            }
        }
        for (name, value, bounds) in [
            (
                "zoom",
                &mut self.zoom,
                Bounds::new(0.01, 10000.0, &[(0.1, 100.0)]),
            ),
            (
                "angle",
                &mut self.angle,
                Bounds::new(-180.0, 180.0, &[(-90.0, 90.0)]),
            ),
            (
                "inversion",
                &mut self.inversion,
                Bounds::new(-8.0, 8.0, &[(-2.0, 2.0)]),
            ),
            (
                "twist",
                &mut self.twist,
                Bounds::new(-180.0, 180.0, &[(-30.0, 30.0)]),
            ),
            (
                "relaxation",
                &mut self.relaxation,
                Bounds::new(0.01, 2.0, &[(0.5, 1.2)]),
            ),
            (
                "memory",
                &mut self.memory,
                Bounds::new(-2.0, 2.0, &[(-0.7, 0.7)]),
            ),
            (
                "bailout",
                &mut self.bailout,
                Bounds::new(2.0, 64.0, &[(4.0, 32.0)]),
            ),
            (
                "trap_radius",
                &mut self.trap_radius,
                Bounds::new(0.0, 8.0, &[(0.1, 2.0)]),
            ),
            (
                "stripes",
                &mut self.stripes,
                Bounds::new(0.0, 128.0, &[(4.0, 32.0)]),
            ),
        ] {
            f(&join(prefix, name), value, bounds);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stripe_colour_is_continuous_when_escape_takes_one_more_iteration() {
        let p = PlanarFractal {
            julia: [0.0, 0.0],
            ..PlanarFractal::default()
        };
        let angle = std::f32::consts::PI / 32.0;
        let direction = Vec2::new(angle.cos(), angle.sin());
        let inside = p.sample(direction * (4.0 - 1e-5));
        let outside = p.sample(direction * (4.0 + 1e-5));
        assert_ne!(inside.iterations, outside.iterations);
        assert!(
            (inside.traps[2] - outside.traps[2]).abs() < 1e-4,
            "{} vs {}",
            inside.traps[2],
            outside.traps[2]
        );
        assert!((inside.traps[0] - outside.traps[0]).abs() < 1e-5);
    }
    #[test]
    fn polynomial_matches_complex_algebra_without_spherical_folding() {
        assert_eq!(
            complex_power(Vec2::new(-1.0, 2.0), 2),
            Vec2::new(-3.0, -4.0)
        );
        assert_eq!(
            complex_power(Vec2::new(-1.0, 2.0), 3),
            Vec2::new(11.0, -2.0)
        );
    }
    #[test]
    fn classical_orbits_and_root_are_reproducible() {
        let p = PlanarFractal {
            family: PlanarFamily::Mandelbrot,
            ..PlanarFractal::default()
        };
        assert_eq!(p.sample(Vec2::ZERO).iterations, p.iterations);
        assert!(p.sample(Vec2::new(2.0, 1.0)).iterations < 5);
        let n = PlanarFractal {
            family: PlanarFamily::Newton,
            power: 3,
            ..p.clone()
        };
        assert_eq!(n.sample(Vec2::X).iterations, 0);
        for family in [
            PlanarFamily::Julia,
            PlanarFamily::Mandelbrot,
            PlanarFamily::BurningShip,
            PlanarFamily::Newton,
            PlanarFamily::Nova,
            PlanarFamily::Phoenix,
        ] {
            let q = PlanarFractal {
                family,
                ..p.clone()
            };
            for i in 0..100 {
                let v = q.sample(Vec2::new(
                    (i % 10) as f32 * 0.37 - 1.6,
                    (i / 10) as f32 * 0.31 - 1.5,
                ));
                assert!(v.distance.is_finite());
                assert!(
                    v.traps
                        .iter()
                        .all(|x| x.is_finite() && (0.0..=1.0).contains(x)),
                    "{family:?}: {v:?}"
                );
            }
        }
    }
}
