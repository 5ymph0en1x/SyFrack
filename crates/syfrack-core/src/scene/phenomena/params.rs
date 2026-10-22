use super::*;
use crate::params::{join, visit_u32, visit_vec3, Bounds, Params};

const SIGNED: Bounds = Bounds::new(-10000.0, 10000.0, &[]);
const POSITIVE: Bounds = Bounds::new(0.00001, 10000.0, &[]);
const NONNEGATIVE: Bounds = Bounds::new(0.0, 10000.0, &[]);
const UNIT: Bounds = Bounds::new(0.0, 1.0, &[]);
impl Params for Phenomena {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        f(&join(prefix, "time"), &mut self.time, NONNEGATIVE);
        visit_vec3(
            &join(prefix, "domain.center"),
            &mut self.domain.center,
            SIGNED,
            f,
        );
        visit_vec3(
            &join(prefix, "domain.half"),
            &mut self.domain.half,
            POSITIVE,
            f,
        );
        for (i, v) in self.domain.resolution.iter_mut().enumerate() {
            visit_u32(
                &join(prefix, &format!("domain.resolution[{i}]")),
                v,
                Bounds::new(if i < 2 { 2.0 } else { 1.0 }, 256.0, &[]),
                f,
            );
        }
        visit_u32(
            &join(prefix, "domain.memory_budget_mib"),
            &mut self.domain.memory_budget_mib,
            Bounds::new(8.0, 2048.0, &[]),
            f,
        );
        f(
            &join(prefix, "bindings.isovalue"),
            &mut self.bindings.isovalue,
            SIGNED,
        );
        f(
            &join(prefix, "bindings.displacement_amount"),
            &mut self.bindings.displacement_amount,
            SIGNED,
        );
        f(
            &join(prefix, "bindings.emission_gain"),
            &mut self.bindings.emission_gain,
            NONNEGATIVE,
        );
        f(
            &join(prefix, "bindings.density_gain"),
            &mut self.bindings.density_gain,
            NONNEGATIVE,
        );
        self.bindings
            .palette
            .visit_params(&join(prefix, "bindings.palette"), f);
        for node in &mut self.fields.nodes {
            let p = join(prefix, &format!("fields.nodes[{}]", node.id));
            match &mut node.op {
                FieldOp::Planar { fractal, channels } => {
                    fractal.visit_params(&join(&p, "pattern"), f);
                    channels.visit_params(&join(&p, "channels"), f);
                }
                FieldOp::Texture { texture } => texture.visit_params(&join(&p, "texture"), f),
                FieldOp::Palette { ramp } => ramp.visit_params(&join(&p, "palette"), f),
                FieldOp::Scalar { value } => f(&join(&p, "value"), value, SIGNED),
                FieldOp::Vector { value } => visit_vec3(&join(&p, "value"), value, SIGNED, f),
                FieldOp::Complex { value } => {
                    f(&join(&p, "real"), &mut value[0], SIGNED);
                    f(&join(&p, "imaginary"), &mut value[1], SIGNED);
                }
                FieldOp::Clamp { min, max } => {
                    f(&join(&p, "min"), min, SIGNED);
                    f(&join(&p, "max"), max, SIGNED);
                }
                FieldOp::Power { exponent } => f(
                    &join(&p, "exponent"),
                    exponent,
                    Bounds::new(-32.0, 32.0, &[]),
                ),
                FieldOp::ComplexPower { power } => {
                    visit_u32(&join(&p, "power"), power, Bounds::new(1.0, 32.0, &[]), f)
                }
                FieldOp::PlaneWave {
                    wavevector,
                    frequency,
                    phase,
                } => {
                    visit_vec3(&join(&p, "wavevector"), wavevector, SIGNED, f);
                    f(&join(&p, "frequency"), frequency, SIGNED);
                    f(&join(&p, "phase"), phase, SIGNED);
                }
                FieldOp::GaussianWave {
                    center,
                    width,
                    wavevector,
                    frequency,
                } => {
                    visit_vec3(&join(&p, "center"), center, SIGNED, f);
                    visit_vec3(&join(&p, "wavevector"), wavevector, SIGNED, f);
                    f(&join(&p, "width"), width, POSITIVE);
                    f(&join(&p, "frequency"), frequency, SIGNED);
                }
                FieldOp::SphereDistance { center, radius } => {
                    visit_vec3(&join(&p, "center"), center, SIGNED, f);
                    f(&join(&p, "radius"), radius, POSITIVE);
                }
                FieldOp::QuaternionJulia {
                    constant,
                    slice,
                    iterations,
                    bailout,
                } => {
                    for (j, c) in constant.iter_mut().enumerate() {
                        f(&join(&p, &format!("constant[{j}]")), c, SIGNED);
                    }
                    f(&join(&p, "slice"), slice, SIGNED);
                    visit_u32(
                        &join(&p, "iterations"),
                        iterations,
                        Bounds::new(1.0, 128.0, &[]),
                        f,
                    );
                    f(
                        &join(&p, "bailout"),
                        bailout,
                        Bounds::new(1.0001, 1000.0, &[]),
                    );
                }
                FieldOp::Mobius { a, b, c, d } => {
                    for (name, value) in [("a", a), ("b", b), ("c", c), ("d", d)] {
                        f(&join(&p, &format!("{name}.real")), &mut value[0], SIGNED);
                        f(
                            &join(&p, &format!("{name}.imaginary")),
                            &mut value[1],
                            SIGNED,
                        );
                    }
                }
                _ => {}
            }
        }
        for (i, s) in self.simulations.iter_mut().enumerate() {
            let p = join(prefix, &format!("simulations[{i}]"));
            visit_u32(
                &join(&p, "steps"),
                &mut s.steps,
                Bounds::new(0.0, 16384.0, &[]),
                f,
            );
            f(&join(&p, "dt"), &mut s.dt, Bounds::new(0.000001, 10.0, &[]));
            match &mut s.model {
                SimulationModel::GrayScott {
                    diffusion,
                    feed,
                    kill,
                } => {
                    for (j, d) in diffusion.iter_mut().enumerate() {
                        f(&join(&p, &format!("diffusion[{j}]")), d, POSITIVE);
                    }
                    f(&join(&p, "feed"), feed, UNIT);
                    f(&join(&p, "kill"), kill, UNIT);
                }
                SimulationModel::GrossPitaevskii {
                    dispersion,
                    interaction,
                    damping,
                } => {
                    f(&join(&p, "dispersion"), dispersion, POSITIVE);
                    f(&join(&p, "interaction"), interaction, SIGNED);
                    f(&join(&p, "damping"), damping, UNIT);
                }
                SimulationModel::DielectricGrowth {
                    eta,
                    relaxation_steps,
                    branches_per_step,
                } => {
                    f(&join(&p, "eta"), eta, Bounds::new(0.0, 16.0, &[]));
                    visit_u32(
                        &join(&p, "relaxation_steps"),
                        relaxation_steps,
                        Bounds::new(1.0, 512.0, &[]),
                        f,
                    );
                    visit_u32(
                        &join(&p, "branches_per_step"),
                        branches_per_step,
                        Bounds::new(1.0, 64.0, &[]),
                        f,
                    );
                }
                SimulationModel::Resonance { mode, iterations } => {
                    visit_u32(&join(&p, "mode"), mode, Bounds::new(0.0, 15.0, &[]), f);
                    visit_u32(
                        &join(&p, "iterations"),
                        iterations,
                        Bounds::new(8.0, 4096.0, &[]),
                        f,
                    );
                }
                SimulationModel::Schottky {
                    pairs,
                    points,
                    burn_in,
                } => {
                    visit_u32(
                        &join(&p, "points"),
                        points,
                        Bounds::new(1.0, 4_000_000.0, &[]),
                        f,
                    );
                    visit_u32(
                        &join(&p, "burn_in"),
                        burn_in,
                        Bounds::new(0.0, 4096.0, &[]),
                        f,
                    );
                    for (j, pair) in pairs.iter_mut().enumerate() {
                        f(
                            &join(&p, &format!("pairs[{j}].radius")),
                            &mut pair.radius,
                            POSITIVE,
                        );
                        f(
                            &join(&p, &format!("pairs[{j}].twist")),
                            &mut pair.twist,
                            Bounds::new(-180.0, 180.0, &[]),
                        );
                    }
                }
            }
            for (j, seed) in s.seeds.iter_mut().enumerate() {
                let p = join(&p, &format!("seeds[{j}]"));
                visit_vec3(&join(&p, "center"), &mut seed.center, SIGNED, f);
                f(&join(&p, "radius"), &mut seed.radius, POSITIVE);
                f(&join(&p, "amplitude"), &mut seed.amplitude, NONNEGATIVE);
                f(&join(&p, "phase"), &mut seed.phase, SIGNED);
            }
        }
        if let Some(d) = &mut self.dispersion {
            f(
                &join(prefix, "dispersion.cauchy_b"),
                &mut d.cauchy_b,
                Bounds::new(0.0, 0.02, &[]),
            );
        }
        if let Some(film) = &mut self.film {
            for (name, value, b) in [
                ("thickness_nm", &mut film.thickness_nm, NONNEGATIVE),
                ("ior", &mut film.ior, Bounds::new(1.0, 4.0, &[])),
                ("substrate_ior", &mut film.substrate_ior, POSITIVE),
                (
                    "substrate_extinction",
                    &mut film.substrate_extinction,
                    NONNEGATIVE,
                ),
                ("strength", &mut film.strength, UNIT),
                (
                    "thickness_range_nm",
                    &mut film.thickness_range_nm,
                    NONNEGATIVE,
                ),
                ("dispersion_b", &mut film.dispersion_b, UNIT),
            ] {
                f(&join(prefix, &format!("film.{name}")), value, b);
            }
        }
        if let Some(lens) = &mut self.lens {
            visit_vec3(&join(prefix, "lens.center"), &mut lens.center, SIGNED, f);
            visit_vec3(&join(prefix, "lens.axis"), &mut lens.axis, SIGNED, f);
            f(&join(prefix, "lens.mass"), &mut lens.mass, NONNEGATIVE);
            f(
                &join(prefix, "lens.spin"),
                &mut lens.spin,
                Bounds::new(-0.999, 0.999, &[]),
            );
            f(
                &join(prefix, "lens.step"),
                &mut lens.step,
                Bounds::new(0.00001, 1.0, &[]),
            );
            f(
                &join(prefix, "lens.escape_radius"),
                &mut lens.escape_radius,
                POSITIVE,
            );
            visit_u32(
                &join(prefix, "lens.max_steps"),
                &mut lens.max_steps,
                Bounds::new(16.0, 16384.0, &[]),
                f,
            );
        }
        for (i, g) in self.guides.iter_mut().enumerate() {
            let p = join(prefix, &format!("guides[{i}]"));
            visit_vec3(&join(&p, "center"), &mut g.center, SIGNED, f);
            f(&join(&p, "radius"), &mut g.radius, POSITIVE);
            f(&join(&p, "strength"), &mut g.strength, NONNEGATIVE);
        }
    }
}
