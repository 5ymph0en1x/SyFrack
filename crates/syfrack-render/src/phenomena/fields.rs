use super::grid::Grid;
use anyhow::{bail, ensure, Result};
use glam::{Vec2, Vec3, Vec4};
use syfrack_core::scene::phenomena::*;
use syfrack_core::Scene;

pub struct Evaluator<'a> {
    pub scene: &'a Scene,
    pub fields: &'a FieldGraph,
    pub snapshots: &'a [Grid],
    order: Vec<usize>,
    epsilon: Vec3,
}
fn cmul(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x)
}
impl<'a> Evaluator<'a> {
    pub fn new(scene: &'a Scene, p: &'a Phenomena, snapshots: &'a [Grid]) -> Result<Self> {
        let order = p
            .fields
            .order()?
            .iter()
            .map(|id| p.fields.nodes.iter().position(|n| n.id == *id).unwrap())
            .collect();
        let r = p.domain.resolution;
        let epsilon = p.domain.half * 2.0
            / Vec3::new(r[0].max(2) as f32, r[1].max(2) as f32, r[2].max(2) as f32)
            * 0.5;
        Ok(Self {
            scene,
            fields: &p.fields,
            snapshots,
            order,
            epsilon,
        })
    }
    pub fn sample(&self, id: FieldId, p: Vec3) -> Result<Vec4> {
        let index = self
            .fields
            .nodes
            .iter()
            .position(|n| n.id == id)
            .ok_or_else(|| anyhow::anyhow!("missing field {id}"))?;
        let mut memo = [None; MAX_FIELD_NODES];
        self.eval(index, p, &mut memo, 0)
    }
    /// Render roots share memoized ancestors. Unbound editor nodes do no work.
    pub fn sample_outputs<const N: usize>(
        &self,
        ids: [Option<FieldId>; N],
        p: Vec3,
    ) -> Result<[Vec4; N]> {
        let mut memo = [None; MAX_FIELD_NODES];
        let mut values = [Vec4::ZERO; N];
        for (slot, id) in ids.into_iter().enumerate() {
            if let Some(id) = id {
                let index = self
                    .fields
                    .nodes
                    .iter()
                    .position(|n| n.id == id)
                    .ok_or_else(|| anyhow::anyhow!("missing field {id}"))?;
                values[slot] = self.eval(index, p, &mut memo, 0)?;
            }
        }
        Ok(values)
    }
    fn eval(
        &self,
        index: usize,
        p: Vec3,
        memo: &mut [Option<Vec4>; MAX_FIELD_NODES],
        differentials: u32,
    ) -> Result<Vec4> {
        if let Some(v) = memo[index] {
            return Ok(v);
        }
        let node = &self.fields.nodes[index];
        let mut input = [Vec4::ZERO; 3];
        for (i, id) in node.inputs.iter().enumerate() {
            let ix = self.fields.nodes.iter().position(|n| n.id == *id).unwrap();
            input[i] = self.eval(ix, p, memo, differentials)?;
        }
        let a = input[0];
        let b = input[1];
        let scalar = |x: f32| Vec4::new(x, 0.0, 0.0, 0.0);
        let complex = |z: Vec2| Vec4::new(z.x, z.y, 0.0, 0.0);
        let time = self.scene.phenomena.as_ref().map_or(0.0, |p| p.time);
        let value = match &node.op {
            FieldOp::Planar { fractal, channels } => {
                let s = fractal.sample(a.truncate().truncate());
                scalar(channels.value(
                    s.traps,
                    s.iterations as f32 / fractal.iterations.max(1) as f32,
                    0.0,
                ))
            }
            FieldOp::Texture { texture } => scalar(texture.value_at(a.truncate())),
            FieldOp::Flame { source, channel } => {
                let f = &self.scene.phenomena.as_ref().unwrap().flames[*source as usize];
                let grid = f
                    .frozen
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("GPU flame resource has not been prepared"))?;
                let q = a.truncate().truncate() - Vec2::from_array(f.center);
                let (sn, cs) = f.angle.to_radians().sin_cos();
                let q = Vec2::new(cs * q.x + sn * q.y, -sn * q.x + cs * q.y)
                    / Vec2::from_array(f.half)
                    * 0.5
                    + Vec2::splat(0.5);
                let v = if q.min_element() < 0.0 || q.max_element() > 1.0 {
                    Vec4::ZERO
                } else {
                    let r = f.resolution;
                    let p = (Vec2::new(q.x, 1.0 - q.y) * r as f32 - Vec2::splat(0.5))
                        .clamp(Vec2::ZERO, Vec2::splat((r - 1) as f32));
                    let x = p.x.floor() as u32;
                    let y = p.y.floor() as u32;
                    let x1 = (x + 1).min(r - 1);
                    let y1 = (y + 1).min(r - 1);
                    let at = |x, y| Vec4::from_array(grid.values[(y * r + x) as usize]);
                    at(x, y)
                        .lerp(at(x1, y), p.x.fract())
                        .lerp(at(x, y1).lerp(at(x1, y1), p.x.fract()), p.y.fract())
                };
                if *channel == FlameChannel::Color {
                    v.truncate().extend(0.0)
                } else {
                    scalar(v.w)
                }
            }
            FieldOp::Palette { ramp } => Vec3::from_array(ramp.sample(a.x)).extend(0.0),
            FieldOp::Mix => a.lerp(b, input[2].x.clamp(0.0, 1.0)),
            FieldOp::Luminance => scalar(a.truncate().dot(Vec3::new(0.2126, 0.7152, 0.0722))),
            FieldOp::Position => p.extend(0.0),
            FieldOp::Time => scalar(time),
            FieldOp::Scalar { value } => scalar(*value),
            FieldOp::Vector { value } => value.extend(0.0),
            FieldOp::Complex { value } => complex(Vec2::from_array(*value)),
            FieldOp::Add => a + b,
            FieldOp::Subtract => a - b,
            FieldOp::Multiply => a * b,
            FieldOp::Scale => a * b.x,
            FieldOp::Dot => scalar(a.truncate().dot(b.truncate())),
            FieldOp::Cross => a.truncate().cross(b.truncate()).extend(0.0),
            FieldOp::Length => scalar(a.truncate().length()),
            FieldOp::Normalize => a.truncate().normalize_or_zero().extend(0.0),
            FieldOp::Sine => scalar(a.x.sin()),
            FieldOp::Cosine => scalar(a.x.cos()),
            FieldOp::Exponential => scalar(a.x.exp()),
            FieldOp::Absolute => scalar(a.x.abs()),
            FieldOp::Clamp { min, max } => scalar(a.x.clamp(*min, *max)),
            FieldOp::Power { exponent } => scalar(a.x.powf(*exponent)),
            FieldOp::ComplexFromParts => complex(Vec2::new(a.x, b.x)),
            FieldOp::ComplexProduct => complex(cmul(Vec2::new(a.x, a.y), Vec2::new(b.x, b.y))),
            FieldOp::ComplexPower { power } => {
                let mut z = Vec2::X;
                for _ in 0..*power {
                    z = cmul(z, Vec2::new(a.x, a.y));
                }
                complex(z)
            }
            FieldOp::Phase => scalar(a.y.atan2(a.x) / std::f32::consts::TAU + 0.5),
            FieldOp::Intensity => scalar(a.x * a.x + a.y * a.y),
            FieldOp::PlaneWave {
                wavevector,
                frequency,
                phase,
            } => {
                let phase = wavevector.dot(a.truncate()) - *frequency * time + *phase;
                complex(Vec2::new(phase.cos(), phase.sin()))
            }
            FieldOp::GaussianWave {
                center,
                width,
                wavevector,
                frequency,
            } => {
                let p = a.truncate() - *center;
                let envelope = (-p.length_squared() / (2.0 * width * width)).exp();
                let phase = wavevector.dot(p) - *frequency * time;
                complex(Vec2::new(phase.cos(), phase.sin()) * envelope)
            }
            FieldOp::GeometryDistance => scalar(self.scene.fractal.evaluate(a.truncate()).distance),
            FieldOp::GeometryTrap { slot } => {
                scalar(self.scene.fractal.evaluate(a.truncate()).traps[*slot as usize])
            }
            FieldOp::SphereDistance { center, radius } => {
                scalar(a.truncate().distance(*center) - *radius)
            }
            FieldOp::QuaternionJulia {
                constant,
                slice,
                iterations,
                bailout,
            } => {
                let mut q = a.truncate().extend(*slice);
                let c = Vec4::from_array(*constant);
                let mut dr = 1.0;
                let mut r = q.length();
                for _ in 0..*iterations {
                    if r > *bailout {
                        break;
                    }
                    dr *= 2.0 * r;
                    let v = q.truncate();
                    q = (v * (2.0 * q.w)).extend(q.w * q.w - v.length_squared()) + c;
                    r = q.length();
                }
                // Orbit-derivative distance estimate outside the filled Julia set.
                // Saturating the escape radius at one makes the distant empty margin
                // look like a surface after global Lipschitz normalization. Keep its
                // spatial scale instead. The grid derivative bound still controls
                // marching; this estimator is not an exact Euclidean signed distance.
                let d = if r > *bailout {
                    0.5 * r.ln() * r / dr.max(1e-10)
                } else {
                    -1e-4
                };
                scalar(d)
            }
            FieldOp::Mobius { a: ma, b: mb, c, d } => {
                let z = Vec2::new(a.x, a.y);
                let numerator = cmul(Vec2::from_array(*ma), z) + Vec2::from_array(*mb);
                let den = cmul(Vec2::from_array(*c), z) + Vec2::from_array(*d);
                ensure!(
                    den.length_squared() > 1e-20,
                    "field {}: Möbius pole in sampled domain",
                    node.id
                );
                complex(cmul(numerator, Vec2::new(den.x, -den.y)) / den.length_squared())
            }
            FieldOp::Snapshot {
                simulation,
                channel,
            } => {
                let grid = self
                    .snapshots
                    .get(*simulation as usize)
                    .ok_or_else(|| anyhow::anyhow!("snapshot dependency not ready"))?;
                scalar(if grid.contains(a.truncate()) {
                    grid.sample(a.truncate())[*channel as usize]
                } else {
                    0.0
                })
            }
            FieldOp::Gradient | FieldOp::Divergence | FieldOp::Curl => {
                ensure!(
                    differentials < 2,
                    "field {}: more than two nested differential operators",
                    node.id
                );
                let ix = self
                    .fields
                    .nodes
                    .iter()
                    .position(|n| n.id == node.inputs[0])
                    .unwrap();
                let mut derivatives = [Vec4::ZERO; 3];
                for axis in 0..3 {
                    let mut offset = Vec3::ZERO;
                    offset[axis] = self.epsilon[axis];
                    let plus = self.eval(
                        ix,
                        p + offset,
                        &mut [None; MAX_FIELD_NODES],
                        differentials + 1,
                    )?;
                    let minus = self.eval(
                        ix,
                        p - offset,
                        &mut [None; MAX_FIELD_NODES],
                        differentials + 1,
                    )?;
                    derivatives[axis] = (plus - minus) / (2.0 * self.epsilon[axis]);
                }
                match node.op {
                    FieldOp::Gradient => {
                        Vec3::new(derivatives[0].x, derivatives[1].x, derivatives[2].x).extend(0.0)
                    }
                    FieldOp::Divergence => {
                        scalar(derivatives[0].x + derivatives[1].y + derivatives[2].z)
                    }
                    _ => Vec3::new(
                        derivatives[1].z - derivatives[2].y,
                        derivatives[2].x - derivatives[0].z,
                        derivatives[0].y - derivatives[1].x,
                    )
                    .extend(0.0),
                }
            }
        };
        if !value.is_finite() {
            bail!("field {} ({}): non-finite value at {p}", node.id, node.name);
        }
        memo[index] = Some(value);
        Ok(value)
    }
    pub fn sample_all(&self, p: Vec3) -> Result<Vec<Vec4>> {
        let mut memo = [None; MAX_FIELD_NODES];
        for &i in &self.order {
            self.eval(i, p, &mut memo, 0)?;
        }
        Ok((0..self.fields.nodes.len())
            .map(|i| memo[i].unwrap())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(id: u32, op: FieldOp, inputs: Vec<u32>) -> FieldNode {
        FieldNode {
            id,
            op,
            inputs,
            name: String::new(),
            pos: [0.0; 2],
        }
    }
    #[test]
    fn interference_retains_phase_and_differential_fields_are_consistent() {
        let mut s = Scene::example();
        let mut p = Phenomena::default();
        p.fields.nodes = vec![
            node(0, FieldOp::Position, vec![]),
            node(
                1,
                FieldOp::PlaneWave {
                    wavevector: Vec3::X,
                    frequency: 0.0,
                    phase: 0.0,
                },
                vec![0],
            ),
            node(
                2,
                FieldOp::PlaneWave {
                    wavevector: Vec3::X,
                    frequency: 0.0,
                    phase: std::f32::consts::PI,
                },
                vec![0],
            ),
            node(3, FieldOp::Add, vec![1, 2]),
            node(4, FieldOp::Intensity, vec![3]),
            node(5, FieldOp::Divergence, vec![0]),
            node(6, FieldOp::Curl, vec![0]),
        ];
        p.validate().unwrap();
        s.phenomena = Some(p.clone());
        let e = Evaluator::new(&s, &p, &[]).unwrap();
        assert!(e.sample(4, Vec3::new(0.3, 0.2, 0.1)).unwrap().x < 1e-10);
        assert!((e.sample(5, Vec3::ZERO).unwrap().x - 3.0).abs() < 1e-5);
        assert!(e.sample(6, Vec3::ONE).unwrap().length() < 1e-5);
    }
}
