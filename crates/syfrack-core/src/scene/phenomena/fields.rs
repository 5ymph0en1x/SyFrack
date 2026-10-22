use super::Simulation;
use crate::SceneError;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type FieldId = u32;
pub const MAX_FIELD_NODES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    Scalar,
    Vector,
    Complex,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldNode {
    pub id: FieldId,
    pub name: String,
    pub op: FieldOp,
    #[serde(default)]
    pub inputs: Vec<FieldId>,
    #[serde(default)]
    pub pos: [f32; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FieldOp {
    Position,
    Time,
    Scalar {
        value: f32,
    },
    Vector {
        value: Vec3,
    },
    Complex {
        value: [f32; 2],
    },
    Add,
    Subtract,
    Multiply,
    Scale,
    Dot,
    Cross,
    Length,
    Normalize,
    Sine,
    Cosine,
    Exponential,
    Absolute,
    Clamp {
        min: f32,
        max: f32,
    },
    Power {
        exponent: f32,
    },
    Gradient,
    Divergence,
    Curl,
    ComplexFromParts,
    ComplexProduct,
    ComplexPower {
        power: u32,
    },
    Phase,
    Intensity,
    PlaneWave {
        wavevector: Vec3,
        frequency: f32,
        phase: f32,
    },
    GaussianWave {
        center: Vec3,
        width: f32,
        wavevector: Vec3,
        frequency: f32,
    },
    GeometryDistance,
    GeometryTrap {
        slot: u32,
    },
    SphereDistance {
        center: Vec3,
        radius: f32,
    },
    QuaternionJulia {
        constant: [f32; 4],
        slice: f32,
        iterations: u32,
        bailout: f32,
    },
    /// A complex fractional linear map; determinant must be nonzero.
    Mobius {
        a: [f32; 2],
        b: [f32; 2],
        c: [f32; 2],
        d: [f32; 2],
    },
    Snapshot {
        simulation: u32,
        channel: u32,
    },
    Planar {
        fractal: crate::PlanarFractal,
        channels: crate::OrbitColoring,
    },
    Texture {
        texture: Box<crate::FractalTexture>,
    },
    Flame {
        source: u32,
        channel: super::FlameChannel,
    },
    Palette {
        ramp: crate::Ramp,
    },
    Mix,
    Luminance,
}

impl FieldOp {
    /// Socket contract shared by the creation editor and its validator.
    pub fn signature(&self, first: FieldType) -> (Vec<FieldType>, FieldType) {
        use FieldType::*;
        match self {
            Self::Position => (vec![], Vector),
            Self::Time | Self::Scalar { .. } => (vec![], Scalar),
            Self::Vector { .. } => (vec![], Vector),
            Self::Complex { .. } => (vec![], Complex),
            Self::Add | Self::Subtract | Self::Multiply => (vec![first, first], first),
            Self::Scale => (vec![first, Scalar], first),
            Self::Dot => (vec![Vector, Vector], Scalar),
            Self::Cross => (vec![Vector, Vector], Vector),
            Self::Length | Self::Divergence | Self::Luminance => (vec![Vector], Scalar),
            Self::Normalize | Self::Curl => (vec![Vector], Vector),
            Self::Gradient => (vec![Scalar], Vector),
            Self::ComplexFromParts => (vec![Scalar, Scalar], Complex),
            Self::ComplexProduct => (vec![Complex, Complex], Complex),
            Self::ComplexPower { .. } | Self::Mobius { .. } => (vec![Complex], Complex),
            Self::Phase | Self::Intensity => (vec![Complex], Scalar),
            Self::PlaneWave { .. } | Self::GaussianWave { .. } => (vec![Vector], Complex),
            Self::GeometryDistance
            | Self::GeometryTrap { .. }
            | Self::SphereDistance { .. }
            | Self::QuaternionJulia { .. }
            | Self::Snapshot { .. }
            | Self::Planar { .. }
            | Self::Texture { .. } => (vec![Vector], Scalar),
            Self::Flame { channel, .. } => (
                vec![Vector],
                if *channel == super::FlameChannel::Color {
                    Vector
                } else {
                    Scalar
                },
            ),
            Self::Palette { .. } => (vec![Scalar], Vector),
            Self::Mix => (vec![Vector, Vector, Scalar], Vector),
            _ => (vec![Scalar], Scalar),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FieldGraph {
    pub nodes: Vec<FieldNode>,
}

fn fault(id: u32, message: impl Into<String>) -> SceneError {
    SceneError::Graph(format!("field node {id}: {}", message.into()))
}
impl FieldGraph {
    /// Deterministic topological order. Forward references are supported.
    pub fn order(&self) -> Result<Vec<FieldId>, SceneError> {
        if self.nodes.len() > MAX_FIELD_NODES {
            return Err(fault(0, "field graph exceeds 64 nodes"));
        }
        let mut nodes = BTreeMap::new();
        for n in &self.nodes {
            if nodes.insert(n.id, n).is_some() {
                return Err(fault(n.id, "duplicate id"));
            }
        }
        let mut emitted = BTreeSet::new();
        let mut order = vec![];
        while order.len() < nodes.len() {
            let mut progress = false;
            for (&id, node) in &nodes {
                if emitted.contains(&id) {
                    continue;
                }
                if node.inputs.iter().any(|i| !nodes.contains_key(i)) {
                    return Err(fault(id, "missing input node"));
                }
                if node.inputs.iter().all(|i| emitted.contains(i)) {
                    emitted.insert(id);
                    order.push(id);
                    progress = true;
                }
            }
            if !progress {
                return Err(fault(
                    0,
                    "cyclic field graph: feedback requires a simulation state",
                ));
            }
        }
        Ok(order)
    }
    pub fn validate(
        &self,
        simulations: &[Simulation],
    ) -> Result<BTreeMap<FieldId, FieldType>, SceneError> {
        use FieldType::*;
        let mut types = BTreeMap::new();
        for id in self.order()? {
            let n = self.nodes.iter().find(|n| n.id == id).unwrap();
            let input: Vec<_> = n.inputs.iter().map(|i| types[i]).collect();
            let finite = |values: &[f32]| values.iter().all(|v| v.is_finite());
            let (expected, output) = match &n.op {
                FieldOp::Position => (vec![], Vector),
                FieldOp::Time => (vec![], Scalar),
                FieldOp::Scalar { value } => {
                    if !value.is_finite() {
                        return Err(fault(id, "non-finite constant"));
                    }
                    (vec![], Scalar)
                }
                FieldOp::Vector { value } => {
                    if !value.is_finite() {
                        return Err(fault(id, "non-finite vector"));
                    }
                    (vec![], Vector)
                }
                FieldOp::Complex { value } => {
                    if !finite(value) {
                        return Err(fault(id, "non-finite complex value"));
                    }
                    (vec![], Complex)
                }
                FieldOp::Add | FieldOp::Subtract | FieldOp::Multiply => {
                    let t = input.first().copied().unwrap_or(Scalar);
                    if matches!(n.op, FieldOp::Multiply) && t == Complex {
                        return Err(fault(id, "use ComplexProduct for complex multiplication"));
                    }
                    (vec![t, t], t)
                }
                FieldOp::Scale => {
                    let t = input.first().copied().unwrap_or(Vector);
                    (vec![t, Scalar], t)
                }
                FieldOp::Dot => (vec![Vector, Vector], Scalar),
                FieldOp::Cross => (vec![Vector, Vector], Vector),
                FieldOp::Length => (vec![Vector], Scalar),
                FieldOp::Normalize => (vec![Vector], Vector),
                FieldOp::Sine | FieldOp::Cosine | FieldOp::Exponential | FieldOp::Absolute => {
                    (vec![Scalar], Scalar)
                }
                FieldOp::Clamp { min, max } => {
                    if !finite(&[*min, *max]) || min > max {
                        return Err(fault(id, "invalid clamp interval"));
                    }
                    (vec![Scalar], Scalar)
                }
                FieldOp::Power { exponent } => {
                    if !exponent.is_finite() || exponent.abs() > 32.0 {
                        return Err(fault(id, "invalid exponent"));
                    }
                    (vec![Scalar], Scalar)
                }
                FieldOp::Gradient => (vec![Scalar], Vector),
                FieldOp::Divergence => (vec![Vector], Scalar),
                FieldOp::Curl => (vec![Vector], Vector),
                FieldOp::ComplexFromParts => (vec![Scalar, Scalar], Complex),
                FieldOp::ComplexProduct => (vec![Complex, Complex], Complex),
                FieldOp::ComplexPower { power } => {
                    if !(1..=32).contains(power) {
                        return Err(fault(id, "complex power outside 1..32"));
                    }
                    (vec![Complex], Complex)
                }
                FieldOp::Phase | FieldOp::Intensity => (vec![Complex], Scalar),
                FieldOp::PlaneWave {
                    wavevector,
                    frequency,
                    phase,
                } => {
                    if !wavevector.is_finite() || !finite(&[*frequency, *phase]) {
                        return Err(fault(id, "invalid wave"));
                    }
                    (vec![Vector], Complex)
                }
                FieldOp::GaussianWave {
                    center,
                    width,
                    wavevector,
                    frequency,
                } => {
                    if !center.is_finite()
                        || !wavevector.is_finite()
                        || !finite(&[*width, *frequency])
                        || *width <= 0.0
                    {
                        return Err(fault(id, "invalid wave envelope"));
                    }
                    (vec![Vector], Complex)
                }
                FieldOp::GeometryDistance => (vec![Vector], Scalar),
                FieldOp::GeometryTrap { slot } => {
                    if *slot > 3 {
                        return Err(fault(id, "trap slot outside 0..3"));
                    }
                    (vec![Vector], Scalar)
                }
                FieldOp::SphereDistance { center, radius } => {
                    if !center.is_finite() || !radius.is_finite() || *radius <= 0.0 {
                        return Err(fault(id, "invalid sphere"));
                    }
                    (vec![Vector], Scalar)
                }
                FieldOp::QuaternionJulia {
                    constant,
                    slice,
                    iterations,
                    bailout,
                } => {
                    if !finite(constant)
                        || !finite(&[*slice, *bailout])
                        || *bailout <= 1.0
                        || !(1..=128).contains(iterations)
                    {
                        return Err(fault(id, "invalid quaternion Julia"));
                    }
                    (vec![Vector], Scalar)
                }
                FieldOp::Mobius { a, b, c, d } => {
                    let mul = |x: [f32; 2], y: [f32; 2]| {
                        [x[0] * y[0] - x[1] * y[1], x[0] * y[1] + x[1] * y[0]]
                    };
                    let ad = mul(*a, *d);
                    let bc = mul(*b, *c);
                    if ![a, b, c, d].iter().all(|x| finite(*x))
                        || (ad[0] - bc[0]).hypot(ad[1] - bc[1]) < 1e-8
                    {
                        return Err(fault(id, "singular Möbius map"));
                    }
                    (vec![Complex], Complex)
                }
                FieldOp::Snapshot {
                    simulation,
                    channel,
                } => {
                    let s = simulations
                        .get(*simulation as usize)
                        .ok_or_else(|| fault(id, "missing simulation snapshot"))?;
                    if *channel >= s.channels() {
                        return Err(fault(id, "invalid snapshot channel"));
                    }
                    (vec![Vector], Scalar)
                }
                FieldOp::Planar { fractal, channels } => {
                    crate::params::validate_params(fractal, "pattern")?;
                    crate::params::validate_params(channels, "channels")?;
                    (vec![Vector], Scalar)
                }
                FieldOp::Texture { texture } => {
                    texture.validate("pattern")?;
                    (vec![Vector], Scalar)
                }
                FieldOp::Flame { channel, .. } => (
                    vec![Vector],
                    if *channel == super::FlameChannel::Color {
                        Vector
                    } else {
                        Scalar
                    },
                ),
                FieldOp::Palette { ramp } => {
                    ramp.validate("palette")?;
                    (vec![Scalar], Vector)
                }
                FieldOp::Mix => (vec![Vector, Vector, Scalar], Vector),
                FieldOp::Luminance => (vec![Vector], Scalar),
            };
            if input != expected {
                return Err(fault(
                    id,
                    format!("expected {expected:?}, received {input:?}"),
                ));
            }
            if !finite(&n.pos) {
                return Err(fault(id, "invalid editor position"));
            }
            types.insert(id, output);
        }
        Ok(types)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FieldDomain {
    pub center: Vec3,
    pub half: Vec3,
    pub resolution: [u32; 3],
    pub memory_budget_mib: u32,
}
impl Default for FieldDomain {
    fn default() -> Self {
        Self {
            center: Vec3::ZERO,
            half: Vec3::splat(3.0),
            resolution: [48, 48, 48],
            memory_budget_mib: 128,
        }
    }
}
impl FieldDomain {
    pub fn cells(&self) -> u64 {
        self.resolution.iter().map(|&r| u64::from(r)).product()
    }
    pub fn validate(&self) -> Result<(), SceneError> {
        if !self.center.is_finite()
            || !self.half.is_finite()
            || self.half.min_element() <= 0.0
            || self.resolution.iter().any(|&r| !(1..=256).contains(&r))
            || self.resolution[0] < 2
            || self.resolution[1] < 2
            || !(8..=2048).contains(&self.memory_budget_mib)
        {
            return Err(fault(0, "invalid field domain"));
        }
        if self.cells() * 16 > u64::from(self.memory_budget_mib) * 1024 * 1024 {
            return Err(fault(0, "field grid exceeds memory budget"));
        }
        let lo = self.center - self.half;
        let hi = self.center + self.half;
        for axis in 0..3 {
            if self.resolution[axis] > 1 {
                let h = 2.0 * self.half[axis] / (self.resolution[axis] - 1) as f32;
                if !lo[axis].is_finite()
                    || !hi[axis].is_finite()
                    || !(1.0 / (h * h)).is_finite()
                    || lo[axis] + h <= lo[axis]
                    || hi[axis] - h >= hi[axis]
                {
                    return Err(fault(
                        0,
                        "field cell spacing is not representable at this world position",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldGeometry {
    #[default]
    None,
    Union,
    Intersection,
    Replace,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FieldBindings {
    pub rgb: Option<FieldId>,
    pub color: Option<FieldId>,
    pub emission: Option<FieldId>,
    pub density: Option<FieldId>,
    pub geometry: Option<FieldId>,
    pub displacement: Option<FieldId>,
    pub film_thickness: Option<FieldId>,
    pub geometry_mode: FieldGeometry,
    pub isovalue: f32,
    pub displacement_amount: f32,
    pub emission_gain: f32,
    pub density_gain: f32,
    pub palette: crate::Ramp,
}
impl Default for FieldBindings {
    fn default() -> Self {
        Self {
            rgb: None,
            color: None,
            emission: None,
            density: None,
            geometry: None,
            displacement: None,
            film_thickness: None,
            geometry_mode: FieldGeometry::None,
            isovalue: 0.0,
            displacement_amount: 0.0,
            emission_gain: 1.0,
            density_gain: 1.0,
            palette: crate::Ramp::default(),
        }
    }
}
impl FieldBindings {
    pub fn validate(&self, types: &BTreeMap<FieldId, FieldType>) -> Result<(), SceneError> {
        if self
            .rgb
            .is_some_and(|id| types.get(&id) != Some(&FieldType::Vector))
        {
            return Err(fault(
                self.rgb.unwrap(),
                "RGB output requires a vector field",
            ));
        }
        for id in [
            self.color,
            self.emission,
            self.density,
            self.geometry,
            self.displacement,
            self.film_thickness,
        ]
        .into_iter()
        .flatten()
        {
            if types.get(&id) != Some(&FieldType::Scalar) {
                return Err(fault(id, "render binding requires a scalar field"));
            }
        }
        if self.geometry_mode != FieldGeometry::None && self.geometry.is_none() {
            return Err(fault(0, "missing isosurface field"));
        }
        if ![
            self.isovalue,
            self.displacement_amount,
            self.emission_gain,
            self.density_gain,
        ]
        .iter()
        .all(|v| v.is_finite())
            || self.emission_gain < 0.0
            || self.density_gain < 0.0
        {
            return Err(fault(0, "invalid field binding gains"));
        }
        self.palette.validate("phenomena.bindings.palette")
    }
}
