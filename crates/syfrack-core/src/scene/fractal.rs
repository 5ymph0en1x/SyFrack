use super::graph::Graph;
use crate::params::{Bounds, Params};
use glam::Vec3;
use serde::{Deserialize, Serialize};

/// The geometry being rendered: a form graph, compiled to WGSL by
/// syfrack-render/src/graph_wgsl.rs and interpreted on the CPU by
/// graph/eval.rs. The single variant keeps the scene files' spelling
/// `fractal: Graph((…))`; the classic formulas live in `graphs/` as graphs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Fractal {
    /// A node graph compiled to a distance estimator.
    Graph(Graph),
}

/// One evaluation of a distance field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeSample {
    /// Zero selects the global material; 1..=8 select named branch finishes.
    pub material: u32,
    pub distance: f32,
    /// Orbit traps (four slots), each normalised to [0, 1]; unused slots are 1.
    pub traps: [f32; 4],
    /// Iterations performed before bailout (or the maximum).
    pub iterations: u32,
}

impl From<Graph> for Fractal {
    fn from(g: Graph) -> Self {
        Fractal::Graph(g)
    }
}

impl Fractal {
    pub fn graph(&self) -> &Graph {
        match self {
            Fractal::Graph(g) => g,
        }
    }

    pub fn graph_mut(&mut self) -> &mut Graph {
        match self {
            Fractal::Graph(g) => g,
        }
    }

    /// The form graph (a clone).
    pub fn to_graph(&self) -> Graph {
        self.graph().clone()
    }

    pub fn distance(&self, p: Vec3) -> f32 {
        self.evaluate(p).distance
    }

    /// CPU distance estimate, traps and iteration count of the graph.
    pub fn evaluate(&self, p: Vec3) -> DeSample {
        self.graph().evaluate(p)
    }
}

impl Params for Fractal {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        self.graph_mut().visit_params(prefix, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(seed: &mut u32) -> f32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (*seed >> 8) as f32 / 16_777_216.0
    }

    fn random_point(seed: &mut u32, extent: f32) -> Vec3 {
        Vec3::new(lcg(seed), lcg(seed), lcg(seed)) * (2.0 * extent) - extent
    }

    fn classics() -> [(&'static str, Fractal); 3] {
        ["mandelbulb", "mandelbox", "menger"]
            .map(|n| (n, Fractal::Graph(Graph::bundled(n).unwrap())))
    }

    #[test]
    fn the_sphere_on_its_ground_is_unchanged() {
        let f = Fractal::Graph(Graph::bundled("sphere").unwrap());
        assert!((f.distance(Vec3::new(10.0, 0.0, 0.0)) - 1.0).abs() < 1e-6);
        assert_eq!(f.evaluate(Vec3::new(3.0, 0.0, 0.0)).iterations, 0);
    }

    #[test]
    fn step_scales_lie_in_the_unit_interval() {
        for (name, f) in classics() {
            let s = f.graph().step_scale();
            assert!(s > 0.0 && s <= 1.0, "{name}: {s}");
            assert!(f.graph().iteration_product() >= 2, "{name}");
        }
    }

    #[test]
    fn origin_is_inside_each_fractal_and_far_points_are_outside() {
        for (name, f) in classics() {
            // The Menger origin is the removed central cube; use a point in the solid.
            let inside = if name == "menger" {
                Vec3::splat(0.5)
            } else {
                Vec3::ZERO
            };
            assert!(
                f.distance(inside) <= 0.05,
                "{name} at {inside}: {}",
                f.distance(inside)
            );
            let far = f.distance(Vec3::new(0.0, 0.0, 20.0));
            // Distance estimates are conservative far from the set (the Mandelbox gives about 4.7 at 20).
            assert!(far > 2.0 && far.is_finite(), "{name} far: {far}");
        }
    }

    #[test]
    fn distance_estimates_are_lipschitz_within_the_step_scale() {
        // |DE(p) - DE(q)| * step <= |p - q| * 1.05 on random pairs near the set.
        for (name, f) in classics() {
            let step = f.graph().step_scale();
            let mut seed = 42u32;
            let mut violations = 0;
            for _ in 0..2000 {
                let p = random_point(&mut seed, 2.0);
                let q = p + random_point(&mut seed, 0.05);
                let dp = f.distance(p) * step;
                let dq = f.distance(q) * step;
                if (dp - dq).abs() > (p - q).length() * 1.05 + 1e-5 {
                    violations += 1;
                }
            }
            assert!(
                violations <= 20,
                "{name}: {violations} Lipschitz violations out of 2000"
            );
        }
    }

    #[test]
    fn samples_are_finite_and_trap_is_normalised() {
        for (name, f) in classics() {
            let mut seed = 7u32;
            for _ in 0..500 {
                let s = f.evaluate(random_point(&mut seed, 3.0));
                assert!(s.distance.is_finite(), "{name}");
                assert!(
                    (0.0..=1.0).contains(&s.traps[0]),
                    "{name} trap {}",
                    s.traps[0]
                );
                assert!(s.iterations <= f.graph().iteration_product());
            }
        }
    }

    #[test]
    fn mandelbulb_is_symmetric_under_z_rotation_by_two_pi_over_power_minus_one() {
        // (Rz)^n + Rc = R(R^(n-1) z^n + c), so the power-n set is invariant under
        // rotation by 2π/(n-1) about z (n = 8 gives 2π/7), not by 2π/n.
        let f = Fractal::Graph(Graph::bundled("mandelbulb").unwrap());
        let p = Vec3::new(0.7, 0.2, 0.3);
        let rot = |angle: f32| {
            let (s, c) = angle.sin_cos();
            Vec3::new(c * p.x - s * p.y, s * p.x + c * p.y, p.z)
        };
        let q = rot(std::f32::consts::TAU / 7.0);
        assert!((f.distance(p) - f.distance(q)).abs() < 1e-4);
        // Negative control: a wrong angle must be detected.
        let q_pi4 = rot(std::f32::consts::FRAC_PI_4);
        assert!((f.distance(p) - f.distance(q_pi4)).abs() > 1e-3);
    }

    #[test]
    fn mandelbox_min_radius_lower_bound_prevents_nan() {
        let mut s = crate::Scene::example_for("mandelbox").unwrap();
        s.set_param("fractal.nodes[4].min_radius", 0.05).unwrap();
        assert!(s.fractal.distance(Vec3::ZERO).is_finite());
        assert!(matches!(
            s.set_param("fractal.nodes[4].min_radius", 0.0),
            Err(crate::SceneError::OutOfBounds { .. })
        ));
    }

    #[test]
    fn params_paths_exist_for_each_fractal() {
        for (name, expected) in [
            ("mandelbulb", "fractal.nodes[3].power"),
            ("mandelbox", "fractal.nodes[4].min_radius"),
            ("menger", "fractal.nodes[2].offset.y"),
        ] {
            let s = crate::Scene::example_for(name).unwrap();
            let paths: Vec<String> = s.params().into_iter().map(|p| p.path).collect();
            assert!(paths.contains(&expected.to_string()), "{paths:?}");
            assert!(paths.iter().any(|p| p.ends_with(".iterations")));
            s.validate().unwrap();
        }
    }
}
