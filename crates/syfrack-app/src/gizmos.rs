//! Geometry of the viewport gizmos: sun compass, area-light handles, fog
//! horizon. Pure maths on the scene and the camera basis; `ui::viewport`
//! draws the result and binds the drags.

use glam::Vec3;
use syfrack_core::{CameraBasis, Light};

use crate::input::project;

/// Compass radius as a fraction of the shorter viewport side, in screen space.
pub const SUN_RING_FRACTION: f32 = 0.35;
/// Smallest and largest area-light side, as the parameter bounds allow.
const AREA_SIDE: (f32, f32) = (0.001, 1000.0);
/// Fog height bounds, as the parameter bounds allow.
const FOG_HEIGHT: (f32, f32) = (-100.0, 100.0);

pub struct SunGizmo {
    pub centre: (f32, f32),
    pub handle: (f32, f32),
    pub radius_px: f32,
    pub in_front: bool,
    /// Where the sun points on the ring when it is not `in_front`.
    pub edge_marker: Option<(f32, f32)>,
}

/// World units per pixel at `depth` ahead of the camera; the field of view is
/// horizontal, so the scale follows the viewport width.
fn world_per_px(basis: &CameraBasis, depth: f32, viewport_px: (f32, f32)) -> f32 {
    depth * basis.tan_half_fov / (viewport_px.0 * 0.5).max(1.0)
}

pub fn sun_gizmo(
    basis: &CameraBasis,
    aspect: f32,
    anchor: Vec3,
    dir: Vec3,
    viewport_px: (f32, f32),
) -> Option<SunGizmo> {
    let centre = project(basis, aspect, anchor)?;
    let radius_px = SUN_RING_FRACTION * viewport_px.0.min(viewport_px.1);
    let depth = (anchor - basis.origin).dot(basis.forward).max(1e-3);
    // Sized so the ring has the same radius in pixels at the anchor's depth.
    let ring_world = world_per_px(basis, depth, viewport_px) * radius_px;
    let d = dir.normalize_or(Vec3::Y);
    let tip = anchor + d * ring_world;
    // Pointing away from the viewer; toward the viewer the handle is on the
    // near side of the compass sphere and is drawn dimmed with an edge marker.
    let in_front = d.dot(basis.forward) > -1e-4 && (tip - basis.origin).dot(basis.forward) > 1e-4;
    let handle = project(basis, aspect, tip).unwrap_or(centre);
    let edge_marker = if in_front {
        None
    } else {
        // The azimuth of the sun as it projects on the screen plane; straight
        // along the view axis it has none, so the marker sits at the top.
        let (mut sx, mut sy) = (d.dot(basis.right), -d.dot(basis.up));
        let len = sx.hypot(sy);
        if len > 1e-4 {
            (sx, sy) = (sx / len, sy / len);
        } else {
            (sx, sy) = (0.0, -1.0);
        }
        Some((
            centre.0 + sx * radius_px / viewport_px.0,
            centre.1 + sy * radius_px / viewport_px.1,
        ))
    };
    Some(SunGizmo {
        centre,
        handle,
        radius_px,
        in_front,
        edge_marker,
    })
}

/// Moves the handle by the drag in screen space: the handle is a point on a
/// sphere seen from the camera, so the new position is the old one plus the
/// pixels, put back on the sphere. The handle stays on its side (toward or
/// away from the viewer); pushed past the rim it folds over to the other side,
/// so it follows the cursor everywhere without a jump.
pub fn drag_sun(
    dir: Vec3,
    basis: &CameraBasis,
    dx_px: f32,
    dy_px: f32,
    viewport_px: (f32, f32),
) -> Vec3 {
    let radius_px = (SUN_RING_FRACTION * viewport_px.0.min(viewport_px.1)).max(1.0);
    let d = dir.normalize_or(Vec3::Y);
    // Position on the ring disc in units of its radius, y up.
    let p = glam::Vec2::new(d.dot(basis.right), d.dot(basis.up))
        + glam::Vec2::new(dx_px, -dy_px) / radius_px;
    let mut side = if d.dot(basis.forward) >= 0.0 {
        1.0
    } else {
        -1.0
    };
    let len = p.length();
    let p = if len > 1.0 {
        // Reflect about the rim: the far side of the sphere comes into view.
        side = -side;
        p / len * (2.0 - len).max(0.0)
    } else {
        p
    };
    let depth = (1.0 - p.length_squared()).max(0.0).sqrt() * side;
    (basis.right * p.x + basis.up * p.y + basis.forward * depth).normalize()
}

/// The renderer's rectangle axes (crates/syfrack-render/src/uniforms.rs):
/// helper Y, or Z when the normal is nearly vertical; u = helper x n; v = n x u.
pub fn area_basis(normal: Vec3) -> (Vec3, Vec3) {
    let n = normal.normalize_or(Vec3::Y);
    let helper = if n.y.abs() > 0.999 { Vec3::Z } else { Vec3::Y };
    let u = helper.cross(n).normalize();
    let v = n.cross(u);
    (u, v)
}

/// Centre -+ u/2 -+ v/2, counter-clockwise seen from the normal side.
pub fn area_corners(light: &Light) -> Option<[Vec3; 4]> {
    let (Light::Area {
        position,
        normal,
        size,
        ..
    }
    | Light::Projector {
        position,
        normal,
        size,
        ..
    }) = light
    else {
        return None;
    };
    let (u, v) = area_basis(*normal);
    let hu = u * (size[0] * 0.5);
    let hv = v * (size[1] * 0.5);
    Some([
        *position - hu - hv,
        *position + hu - hv,
        *position + hu + hv,
        *position - hu + hv,
    ])
}

/// A screen drag as a world displacement in the plane through `at` facing the camera.
pub(crate) fn screen_drag(
    basis: &CameraBasis,
    at: Vec3,
    dx_px: f32,
    dy_px: f32,
    viewport_px: (f32, f32),
) -> Vec3 {
    let depth = (at - basis.origin).dot(basis.forward).max(0.05);
    let step = world_per_px(basis, depth, viewport_px);
    basis.right * (dx_px * step) - basis.up * (dy_px * step)
}

pub fn move_area_in_screen_plane(
    light: &Light,
    basis: &CameraBasis,
    dx_px: f32,
    dy_px: f32,
    viewport_px: (f32, f32),
) -> Option<Light> {
    let (Light::Area { position, .. } | Light::Projector { position, .. }) = light else {
        return None;
    };
    let delta = screen_drag(basis, *position, dx_px, dy_px, viewport_px);
    let mut out = light.clone();
    if let Light::Area { position, .. } | Light::Projector { position, .. } = &mut out {
        *position += delta;
    }
    Some(out)
}

/// Corners are numbered as in `area_corners`. The opposite corner stays put in
/// size terms: the centre is fixed and each side grows by twice the drag
/// projected on the rectangle axis, so the corner follows the cursor.
pub fn resize_area(
    light: &Light,
    corner: usize,
    basis: &CameraBasis,
    dx_px: f32,
    dy_px: f32,
    viewport_px: (f32, f32),
) -> Option<Light> {
    let (Light::Area {
        position,
        normal,
        size,
        ..
    }
    | Light::Projector {
        position,
        normal,
        size,
        ..
    }) = light
    else {
        return None;
    };
    let (u, v) = area_basis(*normal);
    let world = screen_drag(basis, *position, dx_px, dy_px, viewport_px);
    let su = if corner == 1 || corner == 2 {
        1.0
    } else {
        -1.0
    };
    let sv = if corner >= 2 { 1.0 } else { -1.0 };
    let mut out = light.clone();
    if let Light::Area { size: s, .. } | Light::Projector { size: s, .. } = &mut out {
        s[0] = (size[0] + 2.0 * su * world.dot(u)).clamp(AREA_SIDE.0, AREA_SIDE.1);
        s[1] = (size[1] + 2.0 * sv * world.dot(v)).clamp(AREA_SIDE.0, AREA_SIDE.1);
    }
    Some(out)
}

pub fn aim_area_at(light: &Light, target: Vec3) -> Option<Light> {
    let (Light::Area { position, .. } | Light::Projector { position, .. }) = light else {
        return None;
    };
    let n = (target - *position).normalize_or(Vec3::NEG_Y);
    let mut out = light.clone();
    if let Light::Area { normal, .. } | Light::Projector { normal, .. } = &mut out {
        *normal = n;
    }
    Some(out)
}

/// The line where the plane y = height crosses the view: two points of the
/// plane, one to the left and one to the right, at the focus distance ahead.
pub fn fog_horizon(
    basis: &CameraBasis,
    aspect: f32,
    height: f32,
) -> Option<((f32, f32), (f32, f32))> {
    let depth = basis.focus_distance.max(0.5);
    let ahead = basis.origin + basis.forward * depth;
    let mut flat_right = Vec3::new(basis.right.x, 0.0, basis.right.z);
    if flat_right.length() < 1e-4 {
        flat_right = Vec3::X;
    }
    let flat_right = flat_right.normalize();
    let span = depth * basis.tan_half_fov * 4.0;
    let mid = Vec3::new(ahead.x, height, ahead.z);
    Some((
        project(basis, aspect, mid - flat_right * span)?,
        project(basis, aspect, mid + flat_right * span)?,
    ))
}

/// Moves the plane so its line follows the cursor, measured at the focus distance.
pub fn drag_fog_height(
    height: f32,
    basis: &CameraBasis,
    dy_px: f32,
    viewport_px: (f32, f32),
) -> f32 {
    let step = world_per_px(basis, basis.focus_distance.max(0.5), viewport_px);
    (height - dy_px * step / basis.up.y.max(0.2)).clamp(FOG_HEIGHT.0, FOG_HEIGHT.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use syfrack_core::{Camera, CameraMode, Color, Light};

    fn cam() -> Camera {
        Camera {
            mode: CameraMode::Orbit {
                point: Vec3::ZERO,
                distance: 5.0,
                azimuth: 0.0,
                elevation: 0.0,
                roll: 0.0,
            },
            focal_mm: 35.0,
            f_number: 0.0,
            focus_distance: 5.0,
        }
    }

    #[test]
    fn sun_gizmo_puts_an_upward_sun_above_the_anchor() {
        let b = cam().basis();
        let g = sun_gizmo(&b, 16.0 / 9.0, Vec3::ZERO, Vec3::Y, (1600.0, 900.0)).unwrap();
        assert!((g.centre.0 - 0.5).abs() < 1e-4 && (g.centre.1 - 0.5).abs() < 1e-4);
        assert!(
            g.handle.1 < g.centre.1,
            "up is up on screen: {:?}",
            g.handle
        );
        assert!((g.handle.0 - 0.5).abs() < 1e-3);
        assert!(g.in_front);
        assert!((g.radius_px - SUN_RING_FRACTION * 900.0).abs() < 1.0);
        // The handle sits on the drawn ring.
        let dy_px = (g.centre.1 - g.handle.1) * 900.0;
        assert!(
            (dy_px - g.radius_px).abs() < 1.0,
            "{dy_px} vs {}",
            g.radius_px
        );
    }

    #[test]
    fn dragging_the_sun_up_raises_its_elevation_and_keeps_unit_length() {
        let b = cam().basis();
        let d0 = Vec3::new(0.0, 0.3, 1.0).normalize();
        let d1 = drag_sun(d0, &b, 0.0, -40.0, (1600.0, 900.0));
        assert!((d1.length() - 1.0).abs() < 1e-5);
        assert!(d1.y > d0.y);
        let d2 = drag_sun(d0, &b, 40.0, 0.0, (1600.0, 900.0));
        assert!(
            (d2.y - d0.y).abs() < 1e-5,
            "horizontal drag keeps elevation"
        );
        assert!(d2.x > d0.x, "dragging right turns the sun toward +right");
    }

    #[test]
    fn dragging_the_sun_handle_follows_the_cursor_on_both_sides() {
        let b = cam().basis();
        let vp = (1600.0, 900.0);
        let d0 = (-b.forward + b.up * 0.3).normalize();
        let at = |d: Vec3| sun_gizmo(&b, 16.0 / 9.0, Vec3::ZERO, d, vp).unwrap().handle;
        let h0 = at(d0);
        let right = at(drag_sun(d0, &b, 30.0, 0.0, vp));
        assert!(
            right.0 > h0.0,
            "dx > 0 moves the handle right: {h0:?} {right:?}"
        );
        let down = at(drag_sun(d0, &b, 0.0, 30.0, vp));
        assert!(
            down.1 > h0.1,
            "dy > 0 moves the handle down: {h0:?} {down:?}"
        );
        // Pointing away from the viewer (the common, in-front case) behaves the same.
        let far = (b.forward + b.up * 0.3).normalize();
        let f0 = at(far);
        assert!(at(drag_sun(far, &b, 30.0, 0.0, vp)).0 > f0.0);
        assert!(at(drag_sun(far, &b, 0.0, 30.0, vp)).1 > f0.1);
        // Pushed far past the rim it folds over and stays a unit vector.
        let wrapped = drag_sun(far, &b, 0.0, 2000.0, vp);
        assert!((wrapped.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn sun_behind_the_camera_gives_an_edge_marker() {
        let b = cam().basis();
        let g = sun_gizmo(&b, 16.0 / 9.0, Vec3::ZERO, -b.forward, (1600.0, 900.0)).unwrap();
        assert!(!g.in_front);
        assert!(g.edge_marker.is_some());
    }

    fn area() -> Light {
        Light::Area {
            position: Vec3::new(0.0, 2.0, 0.0),
            normal: Vec3::NEG_Y,
            size: [2.0, 1.0],
            color: Color::Kelvin(6500.0),
            intensity: 1.0,
        }
    }

    #[test]
    fn area_corners_span_the_size_and_wind_counter_clockwise_from_the_front() {
        let c = area_corners(&area()).unwrap();
        let w = (c[1] - c[0]).length();
        let h = (c[3] - c[0]).length();
        assert!((w - 2.0).abs() < 1e-5 && (h - 1.0).abs() < 1e-5);
        let n = (c[1] - c[0]).cross(c[3] - c[0]).normalize();
        assert!(
            (n - Vec3::NEG_Y).length() < 1e-5,
            "winding follows the normal: {n}"
        );
        let env = Light::Environment {
            zenith: Color::Kelvin(6500.0),
            horizon: Color::Kelvin(6500.0),
            ground: Color::Kelvin(6500.0),
            intensity: 1.0,
            rotation: 0.0,
        };
        assert!(area_corners(&env).is_none());
    }

    #[test]
    fn area_basis_matches_the_renderer_construction() {
        // Literal values of the engine's rule: helper Y (Z when |n.y| > 0.999),
        // u = helper x n, v = n x u (crates/syfrack-render/src/uniforms.rs).
        let (u, v) = area_basis(Vec3::NEG_Y);
        assert!((u - Vec3::X).length() < 1e-6 && (v - Vec3::Z).length() < 1e-6);
        let (u, v) = area_basis(Vec3::Z);
        assert!((u - Vec3::X).length() < 1e-6 && (v - Vec3::Y).length() < 1e-6);
        let (u, v) = area_basis(Vec3::X);
        assert!((u - Vec3::NEG_Z).length() < 1e-6 && (v - Vec3::Y).length() < 1e-6);
    }

    #[test]
    fn moving_an_area_light_follows_the_screen_plane() {
        let b = cam().basis();
        let moved = move_area_in_screen_plane(&area(), &b, 100.0, 0.0, (1600.0, 900.0)).unwrap();
        let (p0, p1) = match (area(), moved) {
            (Light::Area { position: a, .. }, Light::Area { position: m, .. }) => (a, m),
            _ => unreachable!(),
        };
        let d = p1 - p0;
        assert!(d.length() > 0.0 && d.normalize().dot(b.right) > 0.999);
    }

    #[test]
    fn resizing_by_a_corner_changes_the_size_not_the_centre() {
        let b = cam().basis();
        let r = resize_area(&area(), 1, &b, 60.0, 0.0, (1600.0, 900.0)).unwrap();
        match (area(), r) {
            (
                Light::Area {
                    position: p0,
                    size: s0,
                    ..
                },
                Light::Area {
                    position: p1,
                    size: s1,
                    ..
                },
            ) => {
                assert!((p1 - p0).length() < 1e-5);
                assert!(s1[0] > s0[0], "corner 1 dragged right widens: {s1:?}");
                assert!((s1[1] - s0[1]).abs() < 1e-5, "no vertical component");
                assert!(s1[0] >= 0.001 && s1[1] >= 0.001);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_dragged_corner_follows_the_cursor() {
        let b = cam().basis();
        let vp = (1600.0, 900.0);
        let scr = |l: &Light, k: usize| {
            let c = area_corners(l).unwrap()[k];
            crate::input::project(&b, 16.0 / 9.0, c).unwrap().0 * vp.0
        };
        let a = area();
        let wider = resize_area(&a, 0, &b, 40.0, 0.0, vp).unwrap();
        let (Light::Area { size: s0, .. }, Light::Area { size: s1, .. }) = (&a, &wider) else {
            unreachable!()
        };
        assert!(s1[0] < s0[0], "corner 0 dragged right narrows the width");
        // Corner 1 moves right by about the dragged pixels (the light is
        // above the anchor, so perspective makes it approximate).
        let grown = resize_area(&a, 1, &b, 40.0, 0.0, vp).unwrap();
        let moved = scr(&grown, 1) - scr(&a, 1);
        assert!((moved - 40.0).abs() < 8.0, "moved {moved} px for 40 px");
    }

    #[test]
    fn aiming_sets_the_normal_toward_the_target() {
        let r = aim_area_at(&area(), Vec3::new(1.0, 0.0, 0.0)).unwrap();
        assert!(
            matches!(r, Light::Area { normal, .. } if (normal - Vec3::new(1.0, -2.0, 0.0).normalize()).length() < 1e-5)
        );
    }

    #[test]
    fn fog_horizon_is_horizontal_on_screen_for_a_level_camera() {
        let b = cam().basis();
        let (a, c) = fog_horizon(&b, 16.0 / 9.0, 0.0).unwrap();
        assert!((a.1 - c.1).abs() < 1e-4, "same screen height: {a:?} {c:?}");
        assert!(
            (a.1 - 0.5).abs() < 1e-3,
            "y = 0 passes through the centre for a level camera at y = 0"
        );
        let (lo, _) = fog_horizon(&b, 16.0 / 9.0, -1.0).unwrap();
        assert!(lo.1 > a.1, "a lower plane is lower on screen");
        let h2 = drag_fog_height(0.0, &b, 90.0, (1600.0, 900.0));
        assert!(h2 < 0.0, "dragging down lowers the plane");
        let (after, _) = fog_horizon(&b, 16.0 / 9.0, h2).unwrap();
        assert!(
            (after.1 - a.1 - 90.0 / 900.0).abs() < 5e-3,
            "the line follows the cursor: moved {} of the height",
            after.1 - a.1
        );
    }
}
