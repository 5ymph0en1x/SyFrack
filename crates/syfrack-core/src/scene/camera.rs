use crate::params::{join, visit_vec3, Bounds, Params, ANGLE, POSITION};
use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum CameraMode {
    Free {
        position: Vec3,
        target: Vec3,
        /// Degrees. Positive roll turns the camera up vector toward -right,
        /// so the horizon rotates clockwise as seen from the camera.
        roll: f32,
    },
    /// Orbit around `point`: azimuth and elevation in degrees, distance in scene units.
    Orbit {
        point: Vec3,
        distance: f32,
        azimuth: f32,
        elevation: f32,
        roll: f32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    pub mode: CameraMode,
    /// Focal length in mm, full-frame (36 mm wide sensor) equivalent.
    pub focal_mm: f32,
    /// f-number. 0 means a pinhole (no depth of field).
    pub f_number: f32,
    /// Focus distance in scene units (1 unit = 1 m for lens computations).
    pub focus_distance: f32,
}

const ROLL: Bounds = Bounds::new(-180.0, 180.0, &[(-10.0, 10.0)]);
const DISTANCE: Bounds = Bounds::new(0.001, 1000.0, &[(0.5, 6.0)]);
const ELEVATION: Bounds = Bounds::new(-89.0, 89.0, &[(-30.0, 60.0)]);
const FOCAL: Bounds = Bounds::new(8.0, 1200.0, &[(20.0, 135.0)]);
const F_NUMBER: Bounds = Bounds::new(0.0, 64.0, &[(0.0, 0.0), (1.4, 8.0)]);
const FOCUS: Bounds = Bounds::new(0.001, 10000.0, &[(0.5, 10.0)]);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraBasis {
    pub origin: Vec3,
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    /// tan of half the horizontal field of view (36 mm sensor).
    pub tan_half_fov: f32,
    /// Lens radius in scene units; 0 = pinhole.
    pub lens_radius: f32,
    pub focus_distance: f32,
}

impl Camera {
    pub fn basis(&self) -> CameraBasis {
        let (origin, target, roll) = match &self.mode {
            CameraMode::Free {
                position,
                target,
                roll,
            } => (*position, *target, *roll),
            CameraMode::Orbit {
                point,
                distance,
                azimuth,
                elevation,
                roll,
            } => {
                let az = azimuth.to_radians();
                let el = elevation.to_radians();
                let offset =
                    Vec3::new(el.cos() * az.sin(), el.sin(), el.cos() * az.cos()) * *distance;
                (*point + offset, *point, *roll)
            }
        };
        let forward = (target - origin).normalize_or(Vec3::NEG_Z);
        let world_up = if forward.y.abs() > 0.9999 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let right0 = forward.cross(world_up).normalize();
        let up0 = right0.cross(forward);
        let (s, c) = roll.to_radians().sin_cos();
        let right = right0 * c + up0 * s;
        let up = up0 * c - right0 * s;
        let lens_radius = if self.f_number > 0.0 {
            (self.focal_mm / 1000.0) / (2.0 * self.f_number)
        } else {
            0.0
        };
        CameraBasis {
            origin,
            forward,
            right,
            up,
            tan_half_fov: 18.0 / self.focal_mm,
            lens_radius,
            focus_distance: self.focus_distance,
        }
    }
}

impl Params for Camera {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        match &mut self.mode {
            CameraMode::Free {
                position,
                target,
                roll,
            } => {
                let p = join(prefix, "free");
                visit_vec3(&join(&p, "position"), position, POSITION, f);
                visit_vec3(&join(&p, "target"), target, POSITION, f);
                f(&join(&p, "roll"), roll, ROLL);
            }
            CameraMode::Orbit {
                point,
                distance,
                azimuth,
                elevation,
                roll,
            } => {
                let p = join(prefix, "orbit");
                visit_vec3(&join(&p, "point"), point, POSITION, f);
                f(&join(&p, "distance"), distance, DISTANCE);
                f(&join(&p, "azimuth"), azimuth, ANGLE);
                f(&join(&p, "elevation"), elevation, ELEVATION);
                f(&join(&p, "roll"), roll, ROLL);
            }
        }
        f(&join(prefix, "focal_mm"), &mut self.focal_mm, FOCAL);
        f(&join(prefix, "f_number"), &mut self.f_number, F_NUMBER);
        f(
            &join(prefix, "focus_distance"),
            &mut self.focus_distance,
            FOCUS,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free(position: Vec3, target: Vec3) -> Camera {
        Camera {
            mode: CameraMode::Free {
                position,
                target,
                roll: 0.0,
            },
            focal_mm: 36.0,
            f_number: 0.0,
            focus_distance: 5.0,
        }
    }

    #[test]
    fn basis_is_orthonormal_and_looks_at_target() {
        let b = free(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO).basis();
        assert!((b.forward - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-6);
        assert!(b.forward.dot(b.right).abs() < 1e-6);
        assert!(b.forward.dot(b.up).abs() < 1e-6);
        assert!(b.right.dot(b.up).abs() < 1e-6);
        assert!(
            (b.up.y - 1.0).abs() < 1e-6,
            "up should be world-up when unrolled"
        );
        assert!(
            (b.tan_half_fov - 0.5).abs() < 1e-6,
            "36 mm focal on 36 mm sensor"
        );
        assert_eq!(b.lens_radius, 0.0);
    }

    #[test]
    fn orbit_places_the_camera_at_distance_from_point() {
        let cam = Camera {
            mode: CameraMode::Orbit {
                point: Vec3::new(1.0, 0.0, 0.0),
                distance: 3.0,
                azimuth: 0.0,
                elevation: 0.0,
                roll: 0.0,
            },
            focal_mm: 50.0,
            f_number: 2.0,
            focus_distance: 3.0,
        };
        let b = cam.basis();
        assert!((b.origin - Vec3::new(1.0, 0.0, 3.0)).length() < 1e-5);
        assert!((b.forward - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5);
        assert!(
            (b.lens_radius - 0.0125).abs() < 1e-6,
            "50 mm / (2 * f2) = 12.5 mm"
        );
    }

    #[test]
    fn roll_rotates_up_around_forward_with_a_fixed_sign() {
        let cam = Camera {
            mode: CameraMode::Free {
                position: Vec3::new(0.0, 0.0, 5.0),
                target: Vec3::ZERO,
                roll: 90.0,
            },
            focal_mm: 36.0,
            f_number: 0.0,
            focus_distance: 5.0,
        };
        let b = cam.basis();
        // Looking down -Z, right0 = +X, up0 = +Y. Positive roll turns up toward -right0.
        assert!(
            b.up.y.abs() < 1e-5 && (b.up.x + 1.0).abs() < 1e-5,
            "up {:?}",
            b.up
        );
    }

    #[test]
    fn orbit_elevation_up_to_89_degrees_keeps_right_continuous() {
        let basis_at = |el: f32| {
            Camera {
                mode: CameraMode::Orbit {
                    point: Vec3::ZERO,
                    distance: 3.0,
                    azimuth: 0.0,
                    elevation: el,
                    roll: 0.0,
                },
                focal_mm: 50.0,
                f_number: 0.0,
                focus_distance: 3.0,
            }
            .basis()
        };
        let mut prev = basis_at(80.0).right;
        let mut el = 80.5;
        while el <= 89.0 {
            let r = basis_at(el).right;
            assert!(
                r.dot(prev) > 0.99,
                "right flipped at elevation {el}: {prev:?} -> {r:?}"
            );
            prev = r;
            el += 0.5;
        }
    }

    #[test]
    fn looking_straight_down_does_not_degenerate() {
        let b = free(Vec3::new(0.0, 5.0, 0.0), Vec3::ZERO).basis();
        assert!(b.right.is_finite() && b.up.is_finite());
        assert!(b.right.length() > 0.999);
    }
}
