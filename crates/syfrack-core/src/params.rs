use glam::Vec3;

/// Valid range (hard, validated) and "interesting" sub-ranges (where the
/// flâneur searches). `interesting` must lie inside [min, max].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: f32,
    pub max: f32,
    pub interesting: &'static [(f32, f32)],
}

impl Bounds {
    pub const fn new(min: f32, max: f32, interesting: &'static [(f32, f32)]) -> Self {
        Self {
            min,
            max,
            interesting,
        }
    }

    pub fn contains(&self, v: f32) -> bool {
        v.is_finite() && v >= self.min && v <= self.max
    }
}

/// Visits every numeric parameter of a value with its full path and bounds.
/// One implementation per type serves validation, enumeration and mutation.
pub trait Params {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds));
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamInfo {
    pub path: String,
    pub value: f32,
    pub bounds: Bounds,
}

/// Returns the first parameter of `value` outside its valid range, with
/// paths prefixed by `root`.
pub fn validate_params(
    value: &(impl Params + Clone),
    root: &str,
) -> Result<(), crate::scene::SceneError> {
    let mut copy = value.clone();
    let mut err = None;
    copy.visit_params(root, &mut |path, v, bounds| {
        if err.is_none() && !bounds.contains(*v) {
            err = Some(crate::scene::SceneError::OutOfBounds {
                path: path.to_string(),
                value: *v,
                min: bounds.min,
                max: bounds.max,
            });
        }
    });
    err.map_or(Ok(()), Err)
}

pub fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

pub fn visit_vec3(
    prefix: &str,
    v: &mut Vec3,
    b: Bounds,
    f: &mut dyn FnMut(&str, &mut f32, Bounds),
) {
    f(&join(prefix, "x"), &mut v.x, b);
    f(&join(prefix, "y"), &mut v.y, b);
    f(&join(prefix, "z"), &mut v.z, b);
}

/// Visits an integer parameter through the f32 visitor. Writes are rounded
/// and clamped into the bounds so a mutator can never leave a u32 invalid.
pub fn visit_u32(path: &str, v: &mut u32, b: Bounds, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
    let mut tmp = *v as f32;
    f(path, &mut tmp, b);
    let t = if tmp.is_finite() { tmp } else { b.min };
    *v = t.round().clamp(b.min, b.max) as u32;
}

pub fn visit_rgb(
    prefix: &str,
    c: &mut [f32; 3],
    b: Bounds,
    f: &mut dyn FnMut(&str, &mut f32, Bounds),
) {
    f(&join(prefix, "r"), &mut c[0], b);
    f(&join(prefix, "g"), &mut c[1], b);
    f(&join(prefix, "b"), &mut c[2], b);
}

// Shared bounds.
pub const UNIT: Bounds = Bounds::new(0.0, 1.0, &[(0.0, 1.0)]);
pub const POSITION: Bounds = Bounds::new(-1000.0, 1000.0, &[(-5.0, 5.0)]);
pub const DIRECTION: Bounds = Bounds::new(-1.0, 1.0, &[(-1.0, 1.0)]);
pub const ANGLE: Bounds = Bounds::new(-360.0, 360.0, &[(-180.0, 180.0)]);
pub const KELVIN: Bounds = Bounds::new(1000.0, 40000.0, &[(2000.0, 12000.0)]);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visit_u32_rounds_and_clamps_on_write() {
        let mut v = 8u32;
        let b = Bounds::new(2.0, 64.0, &[(8.0, 16.0)]);
        let mut seen = None;
        visit_u32("it", &mut v, b, &mut |path, x, bounds| {
            seen = Some((path.to_string(), *x, bounds));
            *x = 11.6;
        });
        assert_eq!(seen.unwrap(), ("it".to_string(), 8.0, b));
        assert_eq!(v, 12);
        visit_u32("it", &mut v, b, &mut |_, x, _| *x = -3.0);
        assert_eq!(v, 2, "clamped to the lower bound after rounding");
    }

    #[test]
    fn visit_u32_nan_write_falls_back_to_the_lower_bound() {
        let mut v = 9u32;
        let b = Bounds::new(3.0, 64.0, &[]);
        visit_u32("it", &mut v, b, &mut |_, x, _| *x = f32::NAN);
        assert_eq!(v, 3);
    }
}
