//! A bounded heterogeneous medium, with a particle/grid simulation snapshot.
//! Colours describe gas emission; dust absorbs it. `steps` controls volume
//! quadrature independently of the distance-estimator step budget.
use crate::params::{join, visit_rgb, visit_u32, visit_vec3, Bounds, Params, POSITION, UNIT};
use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Nebula {
    /// Hybrid particle/grid simulation; None retains the analytic field.
    pub particles: Option<Particles>,
    pub center: Vec3,
    /// Half extents in the cloud's rotated local frame.
    pub half_size: Vec3,
    /// Rotation around world Z, in degrees.
    pub angle: f32,
    pub density: f32,
    pub emission: f32,
    pub dust: f32,
    pub scattering: f32,
    pub anisotropy: f32,
    pub turbulence: f32,
    pub detail: f32,
    pub cool_color: [f32; 3],
    pub warm_color: [f32; 3],
    /// Local coordinates relative to `center`, before rotation.
    pub core_position: Vec3,
    pub core_radius: f32,
    pub core_energy: f32,
    /// Density of the deterministic, distant star field. Zero disables it.
    pub stars: f32,
    pub seed: u32,
    pub steps: u32,
}

impl Default for Nebula {
    fn default() -> Self {
        Self {
            particles: Some(Particles::default()),
            center: Vec3::new(4.0, -0.6, 0.0),
            half_size: Vec3::new(28.0, 9.0, 2.8),
            angle: 55.0,
            density: 0.30,
            emission: 1.8,
            dust: 3.0,
            scattering: 0.3,
            anisotropy: 0.35,
            turbulence: 1.2,
            detail: 2.0,
            cool_color: [0.025, 0.20, 0.35],
            warm_color: [1.0, 0.24, 0.055],
            core_position: Vec3::new(6.0, -1.7, -0.6),
            core_radius: 1.9,
            core_energy: 9.0,
            stars: 0.18,
            seed: 407,
            steps: 64,
        }
    }
}

impl Params for Nebula {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        if let Some(p) = &mut self.particles {
            p.visit_params(&join(prefix, "particles"), f);
        }
        f(
            &join(prefix, "scattering"),
            &mut self.scattering,
            Bounds::new(0.0, 4.0, &[(0.0, 1.0)]),
        );
        f(
            &join(prefix, "anisotropy"),
            &mut self.anisotropy,
            Bounds::new(-0.9, 0.9, &[(0.0, 0.6)]),
        );
        visit_vec3(&join(prefix, "center"), &mut self.center, POSITION, f);
        visit_vec3(
            &join(prefix, "half_size"),
            &mut self.half_size,
            Bounds::new(0.01, 500.0, &[(1.0, 30.0)]),
            f,
        );
        visit_vec3(
            &join(prefix, "core_position"),
            &mut self.core_position,
            POSITION,
            f,
        );
        visit_rgb(&join(prefix, "cool_color"), &mut self.cool_color, UNIT, f);
        visit_rgb(&join(prefix, "warm_color"), &mut self.warm_color, UNIT, f);
        f(
            &join(prefix, "angle"),
            &mut self.angle,
            super::graph::ANGLE_DEG,
        );
        for (name, value, bounds) in [
            (
                "density",
                &mut self.density,
                Bounds::new(0.0, 10.0, &[(0.2, 2.0)]),
            ),
            (
                "emission",
                &mut self.emission,
                Bounds::new(0.0, 50.0, &[(0.5, 5.0)]),
            ),
            (
                "dust",
                &mut self.dust,
                Bounds::new(0.0, 20.0, &[(1.0, 6.0)]),
            ),
            (
                "turbulence",
                &mut self.turbulence,
                Bounds::new(0.0, 4.0, &[(0.5, 2.0)]),
            ),
            (
                "detail",
                &mut self.detail,
                Bounds::new(0.1, 4.0, &[(0.5, 2.0)]),
            ),
            (
                "core_radius",
                &mut self.core_radius,
                Bounds::new(0.01, 100.0, &[(1.0, 5.0)]),
            ),
            (
                "core_energy",
                &mut self.core_energy,
                Bounds::new(0.0, 100.0, &[(2.0, 20.0)]),
            ),
            (
                "stars",
                &mut self.stars,
                Bounds::new(0.0, 3.0, &[(0.1, 0.5)]),
            ),
        ] {
            f(&join(prefix, name), value, bounds);
        }
        visit_u32(
            &join(prefix, "seed"),
            &mut self.seed,
            Bounds::new(0.0, 65535.0, &[(0.0, 65535.0)]),
            f,
        );
        visit_u32(
            &join(prefix, "steps"),
            &mut self.steps,
            Bounds::new(8.0, 192.0, &[(32.0, 96.0)]),
            f,
        );
    }
}

/// A deterministic simulation snapshot. Dimensions and particle count bound
/// preparation memory independently of the output image resolution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Particles {
    pub count: u32,
    /// Longest axis of the density grid, with aspect-correct other axes.
    pub resolution: u32,
    pub time: f32,
    pub smoke: bool,
    /// Reconstruct a continuous, advected filament field instead of sampled
    /// particle density. Intended for thin nebular gas and absorbing dust.
    pub filaments: bool,
    /// An irradiated cavity with irregular, branching arms. Used only by the
    /// continuous filament field; the colours remain an artistic palette.
    pub spider: bool,
    /// Branching streams with a dusty rift, independent of the cavity preset.
    /// Takes precedence over `spider` in the continuous filament field.
    pub storm: bool,
    /// Fragmented supernova shell and intersecting, volumetric gas streams.
    /// Takes precedence over the other continuous filament sources.
    pub remnant: bool,
    /// Local emissive excitation where distinct streams meet in 3D.
    /// This changes emitted energy, independently of gas opacity.
    pub convergence: f32,
    /// Emissive excitation of the fragmented forward/reverse shock fronts.
    pub shock: f32,
    /// Expansion radius in the remnant's local frame, in world units.
    pub shock_radius: f32,
    /// Extinction/scattering multiplier of remnant gas. Zero is intrinsically
    /// transparent, additive emission; dust retains its own absorption.
    pub gas_opacity: f32,
    /// Relative Z sampling. Lower values allocate more of the bounded volume
    /// budget to fine XY detail; 1 retains aspect-correct cubic cells.
    pub depth_sampling: f32,
    /// Shader-only, filtered 3D filament detail and synthetic emission-band
    /// separation for the remnant. Zero preserves the previous appearance.
    pub spectral_detail: f32,
    /// World-space standard deviation of an ionization front. Voxel filtering
    /// preserves its integrated density when the grid cannot resolve it.
    pub front_width: f32,
    /// Exponent of the molecular cloud density, independent of gas emission.
    pub dust_clumping: f32,
    /// Amount of dilute gas/tendrils, independent of the molecular cloud.
    pub vapour: f32,
    /// Large-scale intermittency of dilute gas. Zero removes this envelope.
    pub gas_clumping: f32,
    /// World-space falloff of dilute gas around the molecular boundary.
    pub dispersion: f32,
    /// Density of the finite, depth-attenuated blue stellar cluster.
    pub cluster_stars: f32,
    pub buoyancy: f32,
    /// Standard deviation of the continuous smoke nozzle, in world units.
    pub emitter_radius: f32,
    /// Upward inlet speed before pressure projection, in world units/second.
    pub inlet_speed: f32,
    pub wind: Vec3,
    /// Gaussian reconstruction radius, measured in voxel widths.
    pub radius: f32,
    pub dissipation: f32,
    pub shadow_steps: u32,
    /// Strength of the bounded, isotropic light-diffusion approximation.
    pub multiple_scattering: f32,
}

impl Default for Particles {
    fn default() -> Self {
        Self {
            count: 1_000_000,
            resolution: 768,
            time: 3.0,
            smoke: false,
            filaments: false,
            spider: false,
            storm: false,
            remnant: false,
            convergence: 2.0,
            shock: 1.0,
            shock_radius: 7.0,
            gas_opacity: 1.0,
            depth_sampling: 1.0,
            spectral_detail: 0.0,
            front_width: 0.055,
            dust_clumping: 2.0,
            vapour: 1.0,
            gas_clumping: 2.4,
            dispersion: 0.65,
            cluster_stars: 0.0,
            buoyancy: 0.0,
            emitter_radius: 0.42,
            inlet_speed: 1.5,
            wind: Vec3::new(0.5, 0.0, 0.0),
            radius: 0.75,
            dissipation: 0.015,
            shadow_steps: 24,
            multiple_scattering: 0.3,
        }
    }
}

impl Params for Particles {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        visit_u32(
            &join(prefix, "count"),
            &mut self.count,
            Bounds::new(4096.0, 4_000_000.0, &[(200_000.0, 2_000_000.0)]),
            f,
        );
        visit_u32(
            &join(prefix, "resolution"),
            &mut self.resolution,
            Bounds::new(48.0, 1536.0, &[(256.0, 1024.0)]),
            f,
        );
        visit_u32(
            &join(prefix, "shadow_steps"),
            &mut self.shadow_steps,
            Bounds::new(8.0, 64.0, &[(16.0, 32.0)]),
            f,
        );
        visit_vec3(
            &join(prefix, "wind"),
            &mut self.wind,
            Bounds::new(-10.0, 10.0, &[(-2.0, 2.0)]),
            f,
        );
        for (name, value, bounds) in [
            (
                "time",
                &mut self.time,
                Bounds::new(0.0, 16.0, &[(0.0, 8.0)]),
            ),
            (
                "buoyancy",
                &mut self.buoyancy,
                Bounds::new(-4.0, 4.0, &[(0.0, 2.0)]),
            ),
            (
                "emitter_radius",
                &mut self.emitter_radius,
                Bounds::new(0.02, 20.0, &[(0.2, 1.0)]),
            ),
            (
                "inlet_speed",
                &mut self.inlet_speed,
                Bounds::new(0.0, 10.0, &[(0.5, 3.0)]),
            ),
            (
                "radius",
                &mut self.radius,
                Bounds::new(0.5, 2.0, &[(0.5, 1.0)]),
            ),
            ("dissipation", &mut self.dissipation, UNIT),
            (
                "front_width",
                &mut self.front_width,
                Bounds::new(0.005, 0.5, &[(0.02, 0.1)]),
            ),
            (
                "dust_clumping",
                &mut self.dust_clumping,
                Bounds::new(0.5, 6.0, &[(1.0, 3.0)]),
            ),
            (
                "vapour",
                &mut self.vapour,
                Bounds::new(0.0, 3.0, &[(0.5, 1.5)]),
            ),
            (
                "gas_clumping",
                &mut self.gas_clumping,
                Bounds::new(0.0, 4.0, &[(1.0, 3.0)]),
            ),
            (
                "dispersion",
                &mut self.dispersion,
                Bounds::new(0.05, 4.0, &[(0.25, 1.0)]),
            ),
            (
                "cluster_stars",
                &mut self.cluster_stars,
                Bounds::new(0.0, 3.0, &[(0.2, 1.0)]),
            ),
            (
                "convergence",
                &mut self.convergence,
                Bounds::new(0.0, 8.0, &[(0.5, 4.0)]),
            ),
            (
                "shock",
                &mut self.shock,
                Bounds::new(0.0, 4.0, &[(0.25, 2.0)]),
            ),
            (
                "shock_radius",
                &mut self.shock_radius,
                Bounds::new(0.1, 100.0, &[(3.0, 15.0)]),
            ),
            ("multiple_scattering", &mut self.multiple_scattering, UNIT),
            ("gas_opacity", &mut self.gas_opacity, UNIT),
            ("spectral_detail", &mut self.spectral_detail, UNIT),
            (
                "depth_sampling",
                &mut self.depth_sampling,
                Bounds::new(0.15, 1.0, &[(0.3, 1.0)]),
            ),
        ] {
            f(&join(prefix, name), value, bounds);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scene;

    #[test]
    fn cloud_roundtrip_defaults_and_parameter_validation() {
        let mut scene = Scene::example();
        scene.atmosphere.nebula = Some(Nebula::default());
        assert_eq!(Scene::from_ron(&scene.to_ron()).unwrap(), scene);
        assert_eq!(ron::from_str::<Nebula>("()").unwrap(), Nebula::default());
        assert!(
            !ron::from_str::<Particles>("(filaments:true,spider:true)")
                .unwrap()
                .storm
        );
        scene
            .atmosphere
            .nebula
            .as_mut()
            .unwrap()
            .particles
            .as_mut()
            .unwrap()
            .storm = true;
        assert_eq!(Scene::from_ron(&scene.to_ron()).unwrap(), scene);
        scene.set_param("atmosphere.nebula.steps", 96.0).unwrap();
        scene
            .set_param("atmosphere.nebula.particles.resolution", 1024.0)
            .unwrap();
        scene
            .set_param("atmosphere.nebula.particles.count", 2_000_000.0)
            .unwrap();
        assert_eq!(scene.atmosphere.nebula.as_ref().unwrap().steps, 96);
        for (path, value) in [
            ("half_size.x", 0.0),
            ("core_radius", 0.0),
            ("steps", 193.0),
            ("density", f32::NAN),
            ("particles.resolution", 1537.0),
            ("particles.count", 4_000_001.0),
            ("particles.convergence", 8.01),
            ("particles.shock", -0.1),
            ("particles.shock_radius", 0.0),
            ("particles.gas_opacity", 1.01),
            ("particles.depth_sampling", 0.0),
            ("particles.spectral_detail", 1.01),
        ] {
            assert!(scene
                .set_param(&format!("atmosphere.nebula.{path}"), value)
                .is_err());
        }
        assert!(crate::clay_scene(&scene).atmosphere.nebula.is_none());
    }

    #[test]
    fn old_atmospheres_load_without_a_cloud() {
        let old = "(fog_density:0.0,fog_color:(1.0,1.0,1.0),fog_height:0.0,fog_falloff:1.0,anisotropy:0.0,cavity_darkness:0.0)";
        assert!(ron::from_str::<super::super::Atmosphere>(old)
            .unwrap()
            .nebula
            .is_none());
        assert!(ron::from_str::<Nebula>("(unknown:1.0)").is_err());
    }
}
