//! Null geodesics in stationary Cartesian Kerr-Schild coordinates (G=c=1).
//! Numeric metric derivatives and RK4 are reference/validation operations.
use crate::SceneError;
use glam::{Quat, Vec3, Vec4};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RelativisticLens {
    pub center: Vec3,
    pub mass: f32,
    pub spin: f32,
    pub axis: Vec3,
    pub step: f32,
    pub max_steps: u32,
    pub escape_radius: f32,
}
impl Default for RelativisticLens {
    fn default() -> Self {
        Self {
            center: Vec3::ZERO,
            mass: 0.3,
            spin: 0.0,
            axis: Vec3::Y,
            step: 0.03,
            max_steps: 2048,
            escape_radius: 30.0,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct GeodesicState {
    pub position: Vec4,
    pub momentum: Vec4,
}
impl RelativisticLens {
    pub fn validate(&self) -> Result<(), SceneError> {
        if !self.center.is_finite()
            || !self.axis.is_finite()
            || self.axis.length_squared() < 1e-12
            || ![self.mass, self.spin, self.step, self.escape_radius]
                .iter()
                .all(|x| x.is_finite())
            || self.mass < 0.0
            || self.spin.abs() >= 1.0
            || self.step <= 0.0
            || self.step > 1.0
            || !(16..=16384).contains(&self.max_steps)
            || self.escape_radius <= 4.0 * self.mass
        {
            return Err(SceneError::Graph("invalid Kerr/Schwarzschild lens".into()));
        }
        Ok(())
    }
    /// Reconstruct an interaction point from the exported spatial arc length.
    /// Used by picking, which cannot unproject a curved ray as origin+t*direction.
    pub fn point_at_arc(&self, origin: Vec3, direction: Vec3, distance: f32) -> Option<Vec3> {
        if !distance.is_finite() || distance < 0.0 {
            return None;
        }
        if self.mass == 0.0 {
            return Some(origin + direction.normalize() * distance);
        }
        let mut s = self.launch_past(origin, direction);
        let mut travelled = 0.0;
        for _ in 0..self.max_steps.saturating_mul(4) {
            if self.radius(s.position.truncate()) <= self.horizon() * 1.001 {
                return None;
            }
            let speed = self.derivative(s).position.truncate().length().max(1e-5);
            let dt = (self.step
                * 0.5
                * (self.radius(s.position.truncate()) / (4.0 * self.mass)).max(1.0))
            .min((distance - travelled) / speed)
            .max(1e-7);
            let next = self.advance(s, dt);
            let segment = next.position.truncate().distance(s.position.truncate());
            if travelled + segment >= distance {
                return Some(s.position.truncate().lerp(
                    next.position.truncate(),
                    ((distance - travelled) / segment.max(1e-10)).clamp(0.0, 1.0),
                ));
            }
            travelled += segment;
            s = next;
        }
        None
    }
    fn local(&self, p: Vec3) -> Vec3 {
        Quat::from_rotation_arc(self.axis.normalize(), Vec3::Z) * (p - self.center)
    }
    pub fn radius(&self, p: Vec3) -> f32 {
        let p = self.local(p);
        let a = self.mass * self.spin;
        let v = p.length_squared() - a * a;
        ((v + (v * v + 4.0 * a * a * p.z * p.z).sqrt()) * 0.5)
            .max(1e-20)
            .sqrt()
    }
    pub fn horizon(&self) -> f32 {
        self.mass * (1.0 + (1.0 - self.spin * self.spin).sqrt())
    }
    /// Inverse metric = eta - 2H l^mu l^nu; vector.w stores time.
    pub fn metric(&self, p: Vec3) -> (f32, Vec4) {
        if self.mass == 0.0 {
            return (0.0, Vec4::new(0.0, 0.0, 1.0, -1.0));
        }
        let local = self.local(p);
        let a = self.mass * self.spin;
        let r = self.radius(p).max(1e-5 * self.mass.max(1e-3));
        let h = self.mass * r * r * r / (r.powi(4) + a * a * local.z * local.z).max(1e-30);
        let direction = Vec3::new(
            (r * local.x + a * local.y) / (r * r + a * a),
            (r * local.y - a * local.x) / (r * r + a * a),
            local.z / r,
        );
        let direction = Quat::from_rotation_arc(Vec3::Z, self.axis.normalize()) * direction;
        (h, direction.extend(-1.0))
    }
    pub fn hamiltonian(&self, p: Vec3, k: Vec4) -> f32 {
        let (h, l) = self.metric(p);
        0.5 * (k.truncate().length_squared() - k.w * k.w - 2.0 * h * l.dot(k).powi(2))
    }
    /// Camera direction is interpreted in the local Eulerian orthonormal frame.
    pub fn launch(&self, origin: Vec3, direction: Vec3) -> GeodesicState {
        self.launch_oriented(origin, direction, 1.0)
    }
    /// Backward camera/adjoint transport is past directed. In a rotating metric
    /// this is not interchangeable with a future ray pointing away from camera.
    pub fn launch_past(&self, origin: Vec3, direction: Vec3) -> GeodesicState {
        self.launch_oriented(origin, direction, -1.0)
    }
    fn launch_oriented(&self, origin: Vec3, direction: Vec3, time_sign: f32) -> GeodesicState {
        let (h, l) = self.metric(origin);
        let direction = direction.normalize();
        let inv = (1.0 + 2.0 * h).sqrt();
        let physical = direction + l.truncate() * l.truncate().dot(direction) * (1.0 / inv - 1.0);
        let kt = time_sign * inv;
        let spatial = physical - l.truncate() * (time_sign * 2.0 * h / inv);
        let common = kt + l.truncate().dot(spatial);
        let momentum = (spatial + l.truncate() * (2.0 * h * common)).extend(-kt + 2.0 * h * common);
        GeodesicState {
            position: origin.extend(0.0),
            momentum,
        }
    }
    fn derivative(&self, s: GeodesicState) -> GeodesicState {
        let (h, l) = self.metric(s.position.truncate());
        let k = s.momentum;
        let position = Vec4::new(k.x, k.y, k.z, -k.w) - l * (2.0 * h * l.dot(k));
        let eps = (self.mass * 0.0005).max(0.00001);
        let p = s.position.truncate();
        let mut gradient = Vec3::ZERO;
        for axis in 0..3 {
            let mut d = Vec3::ZERO;
            d[axis] = eps;
            gradient[axis] =
                (self.hamiltonian(p + d, k) - self.hamiltonian(p - d, k)) / (2.0 * eps);
        }
        GeodesicState {
            position,
            momentum: (-gradient).extend(0.0),
        }
    }
    pub fn advance(&self, s: GeodesicState, step: f32) -> GeodesicState {
        let add = |a: GeodesicState, b: GeodesicState, t: f32| GeodesicState {
            position: a.position + b.position * t,
            momentum: a.momentum + b.momentum * t,
        };
        let a = self.derivative(s);
        let b = self.derivative(add(s, a, step * 0.5));
        let c = self.derivative(add(s, b, step * 0.5));
        let d = self.derivative(add(s, c, step));
        GeodesicState {
            position: s.position
                + (a.position + 2.0 * b.position + 2.0 * c.position + d.position) * (step / 6.0),
            momentum: s.momentum
                + (a.momentum + 2.0 * b.momentum + 2.0 * c.momentum + d.momentum) * (step / 6.0),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_mass_is_a_straight_null_ray() {
        let lens = RelativisticLens {
            mass: 0.0,
            ..Default::default()
        };
        let origin = Vec3::new(0.0, 0.0, 8.0);
        let dir = Vec3::new(0.3, 0.1, -1.0).normalize();
        let s = lens.launch(origin, dir);
        assert!(lens.hamiltonian(origin, s.momentum).abs() < 1e-6);
        let n = lens.advance(s, 0.5);
        assert!(n.position.truncate().distance(origin + dir * 0.5) < 1e-5);
    }
    #[test]
    fn spinning_metric_launch_is_null_and_stationary_energy_is_conserved() {
        for spin in [0.0, 0.8, -0.8] {
            let lens = RelativisticLens {
                spin,
                ..Default::default()
            };
            let mut s = lens.launch(Vec3::new(2.0, 1.0, 8.0), Vec3::new(-0.2, 0.0, -1.0));
            assert!(lens.hamiltonian(s.position.truncate(), s.momentum).abs() < 2e-6);
            let energy = s.momentum.w;
            for _ in 0..100 {
                s = lens.advance(s, 0.02);
            }
            assert_eq!(energy.to_bits(), s.momentum.w.to_bits());
            assert!(lens.hamiltonian(s.position.truncate(), s.momentum).abs() < 2e-4);
        }
    }
}
