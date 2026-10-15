use glam::Vec3;
use syfrack_core::{Camera, CameraBasis, CameraMode};

pub const ORBIT_DEG_PER_PX: f32 = 0.25;
pub const LOOK_DEG_PER_PX: f32 = 0.15;
pub const ZOOM_PER_NOTCH: f32 = 1.12;
const MAX_PITCH_DEG: f32 = 89.0;

pub fn orbit_drag(cam: &mut Camera, dx_px: f32, dy_px: f32) {
    if let CameraMode::Orbit {
        azimuth, elevation, ..
    } = &mut cam.mode
    {
        *azimuth = (*azimuth + dx_px * ORBIT_DEG_PER_PX).rem_euclid(360.0);
        *elevation = (*elevation - dy_px * ORBIT_DEG_PER_PX).clamp(-MAX_PITCH_DEG, MAX_PITCH_DEG);
    }
}

pub fn orbit_zoom(cam: &mut Camera, notches: f32) {
    if let CameraMode::Orbit { distance, .. } = &mut cam.mode {
        *distance = (*distance * ZOOM_PER_NOTCH.powf(-notches)).clamp(0.001, 1000.0);
    }
}

/// Moves the orbit point in the screen plane so the scene follows the cursor.
/// The field of view is horizontal, so the scale is set by the viewport width.
pub fn orbit_pan(cam: &mut Camera, dx_px: f32, dy_px: f32, viewport_w_px: f32) {
    let basis = cam.basis();
    if let CameraMode::Orbit {
        point, distance, ..
    } = &mut cam.mode
    {
        let units_per_px = *distance * basis.tan_half_fov / (viewport_w_px * 0.5).max(1.0);
        *point -= basis.right * dx_px * units_per_px;
        *point += basis.up * dy_px * units_per_px;
    }
}

pub fn free_look(cam: &mut Camera, dx_px: f32, dy_px: f32) {
    if let CameraMode::Free {
        position, target, ..
    } = &mut cam.mode
    {
        let dir = (*target - *position).normalize_or(Vec3::NEG_Z);
        let dist = (*target - *position).length().max(1e-3);
        let yaw = (-dx_px * LOOK_DEG_PER_PX).to_radians();
        let pitch_now = dir.y.clamp(-1.0, 1.0).asin();
        let pitch = (pitch_now - (dy_px * LOOK_DEG_PER_PX).to_radians())
            .clamp(-MAX_PITCH_DEG.to_radians(), MAX_PITCH_DEG.to_radians());
        let horizontal = Vec3::new(dir.x, 0.0, dir.z).normalize_or(Vec3::NEG_Z);
        let rotated = glam::Quat::from_rotation_y(yaw) * horizontal;
        let new_dir = Vec3::new(
            rotated.x * pitch.cos(),
            pitch.sin(),
            rotated.z * pitch.cos(),
        )
        .normalize();
        *target = *position + new_dir * dist;
    }
}

/// `local` = (right, up, forward) amounts in [-1, 1]; speed scales with the
/// distance to the surface under the cursor so cavities can be entered slowly.
pub fn free_move(cam: &mut Camera, local: Vec3, depth_hint: f32, dt_s: f32) {
    let basis = cam.basis();
    let speed = depth_hint.clamp(0.05, 50.0) * 0.8;
    let local = local.clamp_length_max(1.0);
    let delta =
        (basis.right * local.x + basis.up * local.y + basis.forward * local.z) * speed * dt_s;
    if let CameraMode::Free {
        position, target, ..
    } = &mut cam.mode
    {
        *position += delta;
        *target += delta;
    }
}

pub fn roll(cam: &mut Camera, degrees: f32) {
    match &mut cam.mode {
        CameraMode::Free { roll, .. } | CameraMode::Orbit { roll, .. } => {
            // Wrap into (-180, 180] instead of stopping at the limit.
            let wrapped = (*roll + degrees + 180.0).rem_euclid(360.0) - 180.0;
            *roll = if wrapped == -180.0 { 180.0 } else { wrapped };
        }
    }
}

/// An orbit around `point` seen from where `basis` stands: the eye does not
/// move, the view turns to the point. Elevation is held inside the orbit limits.
pub fn orbit_around(basis: &CameraBasis, point: Vec3, roll: f32) -> CameraMode {
    let d = basis.origin - point;
    let distance = d.length().max(1e-3);
    CameraMode::Orbit {
        point,
        distance,
        azimuth: d.x.atan2(d.z).to_degrees(),
        elevation: (d.y / distance)
            .clamp(-1.0, 1.0)
            .asin()
            .to_degrees()
            .clamp(-MAX_PITCH_DEG, MAX_PITCH_DEG),
        roll,
    }
}

/// Focus distance that puts `point` in focus: the lens focuses on a plane,
/// so the distance is measured along the view axis.
pub fn focus_distance(basis: &CameraBasis, point: Vec3) -> f32 {
    (point - basis.origin).dot(basis.forward)
}

/// Primary ray through (u, v) in [0, 1]^2: the shader's `camera_ray` without
/// jitter or depth of field. The field of view is horizontal (`tan_half_fov`
/// spans the image width; the vertical extent is divided by `aspect`).
pub fn ray_dir(basis: &CameraBasis, aspect: f32, u: f32, v: f32) -> Vec3 {
    let nx = u * 2.0 - 1.0;
    let ny = v * 2.0 - 1.0;
    (basis.forward
        + basis.right * (nx * basis.tan_half_fov)
        + basis.up * (-ny * basis.tan_half_fov / aspect))
        .normalize()
}

/// `depth` is the distance along the ray, as the engine reports it.
pub fn unproject(basis: &CameraBasis, aspect: f32, u: f32, v: f32, depth: f32) -> Vec3 {
    basis.origin + ray_dir(basis, aspect, u, v) * depth
}

pub fn project(basis: &CameraBasis, aspect: f32, p: Vec3) -> Option<(f32, f32)> {
    let d = p - basis.origin;
    let z = d.dot(basis.forward);
    if z <= 1e-6 {
        return None;
    }
    let nx = d.dot(basis.right) / (z * basis.tan_half_fov);
    let ny = -d.dot(basis.up) * aspect / (z * basis.tan_half_fov);
    Some(((nx + 1.0) * 0.5, (ny + 1.0) * 0.5))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use syfrack_core::{Camera, CameraMode};

    fn orbit() -> Camera {
        Camera {
            mode: CameraMode::Orbit {
                point: Vec3::ZERO,
                distance: 4.0,
                azimuth: 10.0,
                elevation: 20.0,
                roll: 0.0,
            },
            focal_mm: 35.0,
            f_number: 0.0,
            focus_distance: 4.0,
        }
    }

    #[test]
    fn orbit_around_keeps_the_eye_and_aims_at_the_point() {
        let mut cam = free();
        let before = cam.basis();
        let point = Vec3::new(0.5, -0.3, 1.0);
        cam.mode = orbit_around(&before, point, 0.0);
        let after = cam.basis();
        assert!(
            (after.origin - before.origin).length() < 1e-4,
            "the eye does not jump"
        );
        assert!((after.forward - (point - before.origin).normalize()).length() < 1e-4);
        let CameraMode::Orbit {
            point: p, distance, ..
        } = cam.mode
        else {
            panic!("orbit_around returns an orbit");
        };
        assert_eq!(p, point);
        assert!((distance - (before.origin - point).length()).abs() < 1e-5);
    }

    #[test]
    fn focus_distance_is_measured_along_the_view_axis() {
        let b = free().basis();
        let on_axis = b.origin + b.forward * 3.0;
        assert!((focus_distance(&b, on_axis) - 3.0).abs() < 1e-5);
        let off_axis = on_axis + b.right * 2.0;
        assert!(
            (focus_distance(&b, off_axis) - 3.0).abs() < 1e-5,
            "the focus is a plane"
        );
    }

    fn free() -> Camera {
        Camera {
            mode: CameraMode::Free {
                position: Vec3::new(0.0, 0.0, 5.0),
                target: Vec3::ZERO,
                roll: 0.0,
            },
            focal_mm: 35.0,
            f_number: 0.0,
            focus_distance: 5.0,
        }
    }

    #[test]
    fn orbit_drag_changes_angles_and_clamps_elevation() {
        let mut c = orbit();
        orbit_drag(&mut c, 40.0, -80.0);
        match c.mode {
            CameraMode::Orbit {
                azimuth, elevation, ..
            } => {
                assert!((azimuth - 20.0).abs() < 1e-4);
                assert!((elevation - 40.0).abs() < 1e-4);
            }
            _ => unreachable!(),
        }
        orbit_drag(&mut c, 0.0, -10_000.0);
        assert!(matches!(c.mode, CameraMode::Orbit { elevation, .. } if elevation == 89.0));
    }

    #[test]
    fn orbit_zoom_is_logarithmic_and_bounded() {
        let mut c = orbit();
        orbit_zoom(&mut c, 1.0);
        assert!(
            matches!(c.mode, CameraMode::Orbit { distance, .. } if (distance - 4.0 / ZOOM_PER_NOTCH).abs() < 1e-5)
        );
        orbit_zoom(&mut c, -1000.0);
        assert!(matches!(c.mode, CameraMode::Orbit { distance, .. } if distance == 1000.0));
    }

    #[test]
    fn orbit_pan_moves_the_point_in_the_screen_plane() {
        let mut c = orbit();
        let before = c.basis();
        orbit_pan(&mut c, 100.0, 0.0, 500.0);
        let after = c.basis();
        let moved = match c.mode {
            CameraMode::Orbit { point, .. } => point,
            _ => unreachable!(),
        };
        // 100 px right moves the point by 100 * d * tan_half / (w / 2) along -right.
        let expected = -before.right * (100.0 * 4.0 * before.tan_half_fov / 250.0);
        assert!((moved - expected).length() < 1e-5, "{moved} vs {expected}");
        assert!(
            (after.forward - before.forward).length() < 1e-5,
            "orientation unchanged"
        );
        // Vertical drag: content follows the cursor, so dragging down moves the point up.
        let mut c = orbit();
        orbit_pan(&mut c, 0.0, 50.0, 500.0);
        let moved = match c.mode {
            CameraMode::Orbit { point, .. } => point,
            _ => unreachable!(),
        };
        let expected = before.up * (50.0 * 4.0 * before.tan_half_fov / 250.0);
        assert!((moved - expected).length() < 1e-5, "{moved} vs {expected}");
    }

    #[test]
    fn project_then_unproject_roundtrips() {
        let c = free();
        let b = c.basis();
        let aspect = 16.0 / 9.0;
        for p in [
            Vec3::new(0.3, -0.2, 1.0),
            Vec3::new(-1.0, 0.5, -2.0),
            Vec3::new(0.0, 0.0, 0.0),
        ] {
            let (u, v) = project(&b, aspect, p).expect("in front");
            let depth = (p - b.origin).length();
            let q = unproject(&b, aspect, u, v, depth);
            assert!((q - p).length() < 1e-4, "{p} -> ({u}, {v}) -> {q}");
        }
        assert!(
            project(&b, aspect, Vec3::new(0.0, 0.0, 9.0)).is_none(),
            "behind the camera"
        );
        let (u, v) = project(&b, aspect, Vec3::ZERO).unwrap();
        assert!(
            (u - 0.5).abs() < 1e-5 && (v - 0.5).abs() < 1e-5,
            "target is the centre"
        );
    }

    #[test]
    fn ray_dir_matches_the_shader_convention() {
        let b = free().basis();
        let d = ray_dir(&b, 2.0, 1.0, 0.0);
        assert!(
            d.x > 0.0 && d.y > 0.0,
            "u = 1 looks right, v = 0 looks up: {d}"
        );
        assert!((ray_dir(&b, 2.0, 0.5, 0.5) - b.forward).length() < 1e-6);
    }

    #[test]
    fn ray_dir_vertical_extent_is_divided_by_aspect() {
        let b = free().basis();
        for aspect in [1.0, 2.0, 16.0 / 9.0] {
            let d = ray_dir(&b, aspect, 1.0, 0.0);
            // forward is -z, right is +x, up is +y: the corner ray has y/x = 1/aspect.
            assert!(
                (d.y / d.x - 1.0 / aspect).abs() < 1e-5,
                "aspect {aspect}: {d}"
            );
            assert!(
                (d.x / -d.z - b.tan_half_fov).abs() < 1e-5,
                "horizontal half-angle"
            );
        }
    }

    #[test]
    fn free_move_diagonal_is_not_faster() {
        let mut a = free();
        free_move(&mut a, Vec3::new(1.0, 0.0, 1.0), 1.0, 1.0);
        let mut b = free();
        free_move(&mut b, Vec3::new(0.0, 0.0, 1.0), 1.0, 1.0);
        let pos = |c: &Camera| match c.mode {
            CameraMode::Free { position, .. } => position,
            _ => unreachable!(),
        };
        let step = |c: &Camera| (pos(c) - Vec3::new(0.0, 0.0, 5.0)).length();
        assert!((step(&a) - step(&b)).abs() < 1e-5);
    }

    #[test]
    fn roll_wraps_instead_of_stopping() {
        let mut c = free();
        roll(&mut c, 170.0);
        roll(&mut c, 20.0);
        assert!(matches!(c.mode, CameraMode::Free { roll, .. } if (roll + 170.0).abs() < 1e-4));
        roll(&mut c, -20.0);
        assert!(matches!(c.mode, CameraMode::Free { roll, .. } if (roll - 170.0).abs() < 1e-4));
    }

    #[test]
    fn free_move_speed_follows_depth_and_look_pitch_clamps() {
        let mut c = free();
        free_move(&mut c, Vec3::new(0.0, 0.0, 1.0), 10.0, 1.0);
        assert!(
            matches!(c.mode, CameraMode::Free { position, .. } if (position.z - (5.0 - 8.0)).abs() < 1e-4)
        );
        let mut c = free();
        free_move(&mut c, Vec3::new(0.0, 0.0, 1.0), 0.0001, 1.0);
        assert!(
            matches!(c.mode, CameraMode::Free { position, .. } if (position.z - (5.0 - 0.04)).abs() < 1e-4)
        );
        let mut c = free();
        free_look(&mut c, 0.0, -100_000.0);
        let b = c.basis();
        assert!(
            b.forward.y < 0.9999 && b.forward.y > 0.98,
            "pitch clamps below vertical: {}",
            b.forward.y
        );
    }
}
