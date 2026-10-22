//! Lower the graph to data for the fixed GPU interpreter. Registers are reused
//! after their last consumer, but each instance template has its own bank:
//! a template call cannot overwrite a live value in its caller.
use std::collections::{HashMap, HashSet};

use anyhow::{bail, ensure, Result};
use bytemuck::{Pod, Zeroable};
use syfrack_core::{Graph, Measure, NodeId, NodeKind, TrapMode};

use crate::graph_wgsl::ParamLayout;

#[repr(u32)]
#[derive(Clone, Copy, Debug)]
pub(super) enum Op {
    Input,
    Copy,
    Translate,
    Rotate,
    Scale,
    ScaleAxes,
    Mirror,
    BoxFold,
    SphereFold,
    BoxFoldAxes,
    SphereInversion,
    AddConstant,
    Abs,
    MengerFold,
    Kaleido,
    Power,
    AddOrigin,
    AddSeed,
    Twist,
    Bend,
    Tile,
    RepeatFinite,
    Trap,
    Sphere,
    Torus,
    Cylinder,
    Box,
    RoundBox,
    Rock,
    Plane,
    NormFence,
    BulbFence,
    Union,
    Intersection,
    Subtraction,
    SmoothUnion,
    SmoothIntersection,
    Material,
    Bounded,
    LoopBegin,
    LoopTest,
    LoopEnd,
    InstanceBegin,
    InstanceNext,
    InstanceAccumulate,
    Call,
    Return,
}

pub(super) const OPS: &[Op] = &[
    Op::Input,
    Op::Copy,
    Op::Translate,
    Op::Rotate,
    Op::Scale,
    Op::ScaleAxes,
    Op::Mirror,
    Op::BoxFold,
    Op::SphereFold,
    Op::BoxFoldAxes,
    Op::SphereInversion,
    Op::AddConstant,
    Op::Abs,
    Op::MengerFold,
    Op::Kaleido,
    Op::Power,
    Op::AddOrigin,
    Op::AddSeed,
    Op::Twist,
    Op::Bend,
    Op::Tile,
    Op::RepeatFinite,
    Op::Trap,
    Op::Sphere,
    Op::Torus,
    Op::Cylinder,
    Op::Box,
    Op::RoundBox,
    Op::Rock,
    Op::Plane,
    Op::NormFence,
    Op::BulbFence,
    Op::Union,
    Op::Intersection,
    Op::Subtraction,
    Op::SmoothUnion,
    Op::SmoothIntersection,
    Op::Material,
    Op::Bounded,
    Op::LoopBegin,
    Op::LoopTest,
    Op::LoopEnd,
    Op::InstanceBegin,
    Op::InstanceNext,
    Op::InstanceAccumulate,
    Op::Call,
    Op::Return,
];

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(super) struct Instruction {
    pub op: u32,
    pub dst: u32,
    pub a: u32,
    pub b: u32,
    pub p: u32,
    pub q: u32,
    pub jump: u32,
    pub extra: u32,
}

impl Instruction {
    fn new(op: Op, dst: u32, a: u32, b: u32, p: u32, q: u32) -> Self {
        Self {
            op: op as u32,
            dst,
            a,
            b,
            p,
            q,
            jump: 0,
            extra: 0,
        }
    }
}

pub(super) struct Program {
    pub instructions: Vec<Instruction>,
    pub layout: ParamLayout,
    pub registers: u32,
    pub frames: u32,
}

#[derive(Default)]
struct Registers {
    next: u32,
    free: Vec<u32>,
}
impl Registers {
    fn alloc(&mut self) -> u32 {
        self.free.pop().unwrap_or_else(|| {
            let r = self.next;
            self.next += 1;
            r
        })
    }
    fn release(&mut self, reg: u32) {
        self.free.push(reg);
    }
}

struct Builder<'a> {
    graph: &'a Graph,
    code: Vec<Instruction>,
    layout: ParamLayout,
    registers: u32,
    functions: Vec<NodeId>,
    calls: Vec<(usize, NodeId)>,
    edges: HashMap<NodeId, Vec<NodeId>>,
}

impl Program {
    pub fn compile(graph: &Graph) -> Result<Self> {
        graph.validate()?;
        let output = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Output)
            .unwrap()
            .id;
        let root = graph.input_of(output, 0).unwrap().0;
        let mut b = Builder {
            graph,
            code: Vec::new(),
            layout: ParamLayout::default(),
            registers: 0,
            functions: vec![root],
            calls: Vec::new(),
            edges: HashMap::new(),
        };
        let mut entries = HashMap::new();
        let mut index = 0;
        while index < b.functions.len() {
            let target = b.functions[index];
            entries.insert(target, b.code.len() as u32);
            b.function(target)?;
            ensure!(
                b.code.len() <= 131_072,
                "control graph exceeds the instruction memory limit"
            );
            index += 1;
        }
        for &(at, target) in &b.calls {
            b.code[at].jump = entries[&target];
        }
        fn depth(
            id: NodeId,
            edges: &HashMap<NodeId, Vec<NodeId>>,
            cache: &mut HashMap<NodeId, u32>,
        ) -> u32 {
            if let Some(&d) = cache.get(&id) {
                return d;
            }
            let d = edges
                .get(&id)
                .into_iter()
                .flatten()
                .map(|&n| 1 + depth(n, edges, cache))
                .max()
                .unwrap_or(0);
            cache.insert(id, d);
            d
        }
        let frames = depth(root, &b.edges, &mut HashMap::new()).max(1);
        ensure!(
            b.layout.len() <= crate::graph_wgsl::PARAM_COUNT,
            "control graph parameter limit exceeded"
        );
        Ok(Self {
            instructions: b.code,
            layout: b.layout,
            registers: b.registers.max(1),
            frames,
        })
    }
}

impl Builder<'_> {
    fn field(&mut self, node: NodeId, field: &'static str, count: usize) -> u32 {
        self.layout
            .index_of(node, field)
            .unwrap_or_else(|| self.layout.add(node, field, count)) as u32
    }
    fn fields(&mut self, node: NodeId, fields: &[(&'static str, usize)]) -> u32 {
        let mut base = 0;
        for (i, &(name, count)) in fields.iter().enumerate() {
            let at = self.field(node, name, count);
            if i == 0 {
                base = at;
            }
        }
        base
    }
    fn input(&self, id: NodeId, port: u32) -> NodeId {
        self.graph.input_of(id, port).unwrap().0
    }
    fn dependencies(&self, id: NodeId) -> Vec<NodeId> {
        let n = self.graph.node(id).unwrap();
        use NodeKind::*;
        match &n.kind {
            Input => vec![],
            Instances { .. } if n.enabled => vec![],
            Bounded { .. } if !n.enabled => vec![self.input(id, 1)],
            k if k.is_point_to_point()
                || k.is_fence()
                || matches!(k, Output | MaterialTag { .. } | Instances { .. }) =>
            {
                vec![self.input(id, 0)]
            }
            _ if !n.enabled => vec![self.input(id, 0)],
            _ => vec![self.input(id, 0), self.input(id, 1)],
        }
    }
    fn function(&mut self, target: NodeId) -> Result<()> {
        let mut needed = HashSet::new();
        let mut stack = vec![target];
        let mut uses: HashMap<NodeId, u32> = HashMap::new();
        while let Some(id) = stack.pop() {
            if needed.insert(id) {
                for src in self.dependencies(id) {
                    *uses.entry(src).or_default() += 1;
                    stack.push(src);
                }
            }
        }
        let mut regs = Registers {
            next: self.registers,
            free: Vec::new(),
        };
        let mut names = HashMap::new();
        for id in self.graph.topo_order()? {
            if !needed.contains(&id) {
                continue;
            }
            let dst = regs.alloc();
            let n = self.graph.node(id).unwrap().clone();
            let deps = self.dependencies(id);
            let a = deps.first().map_or(0, |id| names[id]);
            let b = deps.get(1).map_or(0, |id| names[id]);
            use NodeKind::*;
            if !n.enabled && !matches!(n.kind, Input) {
                if n.kind.is_fence() {
                    let mut ins = Instruction::new(Op::Copy, dst, a, 0, 0, 0);
                    ins.extra = 1;
                    self.code.push(ins);
                } else {
                    self.code.push(Instruction::new(Op::Copy, dst, a, 0, 0, 0));
                }
            } else {
                match &n.kind {
                    Input => self.code.push(Instruction::new(Op::Input, dst, 0, 0, 0, 0)),
                    k if k.is_point_to_point() => {
                        self.code.push(Instruction::new(Op::Copy, dst, a, 0, 0, 0));
                        self.point(id, dst, None, &mut regs)?;
                    }
                    Instances { placements, .. } => {
                        let state = regs.alloc();
                        let local = regs.alloc();
                        let result = regs.alloc();
                        let p = self.layout.placements(id, placements.len()) as u32;
                        let q = self.field(id, "bvh", (2 * placements.len() - 1) * 8);
                        let half = self.fields(id, &[("half", 3), ("placement_count", 1)]);
                        self.code
                            .push(Instruction::new(Op::InstanceBegin, dst, state, 0, 0, 0));
                        let next = self.code.len();
                        let mut ins = Instruction::new(Op::InstanceNext, dst, state, local, p, q);
                        ins.extra = half;
                        self.code.push(ins);
                        let template = self.input(id, 0);
                        let call = self.code.len();
                        self.code
                            .push(Instruction::new(Op::Call, result, local, 0, 0, 0));
                        self.calls.push((call, template));
                        self.edges.entry(target).or_default().push(template);
                        if !self.functions.contains(&template) {
                            self.functions.push(template);
                        }
                        let mut acc =
                            Instruction::new(Op::InstanceAccumulate, dst, result, state, 0, 0);
                        acc.jump = next as u32;
                        self.code.push(acc);
                        self.code[next].jump = self.code.len() as u32;
                        regs.release(state);
                        regs.release(local);
                        regs.release(result);
                    }
                    Bounded { .. } => {
                        let p = self.field(id, "half", 3);
                        self.code
                            .push(Instruction::new(Op::Bounded, dst, a, b, p, 0));
                    }
                    MaterialTag { .. } => {
                        let p = self.field(id, "index", 1);
                        self.code
                            .push(Instruction::new(Op::Material, dst, a, 0, p, 0));
                    }
                    k if k.is_fence() => {
                        let (op, p) = match k {
                            Sphere { .. } => (Op::Sphere, self.field(id, "radius", 1)),
                            Torus { .. } => {
                                (Op::Torus, self.fields(id, &[("radius", 1), ("tube", 1)]))
                            }
                            Cylinder { .. } => (
                                Op::Cylinder,
                                self.fields(id, &[("radius", 1), ("half_height", 1)]),
                            ),
                            Box { .. } => (Op::Box, self.field(id, "half", 3)),
                            RoundBox { .. } => {
                                (Op::RoundBox, self.fields(id, &[("half", 3), ("radius", 1)]))
                            }
                            Rock { .. } => (
                                Op::Rock,
                                self.fields(id, &[("half", 3), ("erosion", 1), ("seed", 1)]),
                            ),
                            Plane { .. } => {
                                (Op::Plane, self.fields(id, &[("normal", 3), ("offset", 1)]))
                            }
                            NormFence { .. } => (Op::NormFence, self.field(id, "inset", 1)),
                            BulbFence => (Op::BulbFence, 0),
                            _ => unreachable!(),
                        };
                        let mut instruction = Instruction::new(op, dst, a, 0, p, 0);
                        instruction.extra = id;
                        self.code.push(instruction);
                    }
                    k => {
                        let op = match k {
                            Union => Op::Union,
                            Intersection => Op::Intersection,
                            Subtraction => Op::Subtraction,
                            SmoothUnion { .. } => Op::SmoothUnion,
                            SmoothIntersection { .. } => Op::SmoothIntersection,
                            _ => bail!("unsupported control node {id}: {k:?}"),
                        };
                        let p = if matches!(k, SmoothUnion { .. } | SmoothIntersection { .. }) {
                            self.field(id, "k", 1)
                        } else {
                            0
                        };
                        self.code.push(Instruction::new(op, dst, a, b, p, 0));
                    }
                }
            }
            names.insert(id, dst);
            for src in deps {
                let remaining = uses.get_mut(&src).unwrap();
                *remaining -= 1;
                if *remaining == 0 && src != target {
                    regs.release(names[&src]);
                }
            }
        }
        self.code
            .push(Instruction::new(Op::Return, 0, names[&target], 0, 0, 0));
        self.registers = regs.next;
        Ok(())
    }

    fn point(
        &mut self,
        id: NodeId,
        dst: u32,
        seed: Option<u32>,
        regs: &mut Registers,
    ) -> Result<()> {
        let n = self.graph.node(id).unwrap().clone();
        if !n.enabled {
            return Ok(());
        }
        use NodeKind::*;
        let mask =
            |x: bool, y: bool, z: bool| u32::from(x) | (u32::from(y) << 1) | (u32::from(z) << 2);
        let (op, p, q) = match n.kind {
            Translate { .. } => (Op::Translate, self.field(id, "offset", 3), 0),
            Rotate { .. } => (Op::Rotate, self.field(id, "angles", 3), 0),
            Scale { .. } => (Op::Scale, self.field(id, "factor", 1), 0),
            ScaleAxes { .. } => (Op::ScaleAxes, self.field(id, "factors", 3), 0),
            Mirror { .. } => (
                Op::Mirror,
                self.fields(id, &[("normal", 3), ("offset", 1)]),
                0,
            ),
            BoxFold { .. } => (Op::BoxFold, self.field(id, "limit", 1), 0),
            SphereFold { .. } => (
                Op::SphereFold,
                self.fields(id, &[("min_radius", 1), ("fixed_radius", 1)]),
                0,
            ),
            BoxFoldAxes { x, y, z, .. } => {
                (Op::BoxFoldAxes, self.field(id, "limit", 3), mask(x, y, z))
            }
            SphereInversion { .. } => (
                Op::SphereInversion,
                self.fields(id, &[("radius", 1), ("min_radius", 1)]),
                0,
            ),
            AddConstant { .. } => (
                Op::AddConstant,
                self.fields(id, &[("multiplier", 3), ("julia", 3)]),
                0,
            ),
            Abs { x, y, z } => (Op::Abs, 0, mask(x, y, z)),
            MengerFold { .. } => (
                Op::MengerFold,
                self.fields(id, &[("scale", 1), ("offset", 3)]),
                0,
            ),
            Kaleido { planes, .. } => (Op::Kaleido, self.field(id, "phase", 1), planes),
            Power { .. } => (Op::Power, self.field(id, "power", 1), 0),
            AddOrigin => (Op::AddOrigin, 0, 0),
            AddSeed { .. } => (Op::AddSeed, self.field(id, "multiplier", 3), 0),
            Twist { .. } => (Op::Twist, self.field(id, "rate", 1), 0),
            Bend { .. } => (Op::Bend, self.field(id, "rate", 1), 0),
            Tile { .. } => (Op::Tile, self.field(id, "period", 3), 0),
            RepeatFinite { .. } => (
                Op::RepeatFinite,
                self.fields(id, &[("period", 3), ("count", 3)]),
                0,
            ),
            Trap {
                slot,
                measure,
                mode,
                ..
            } => {
                let measure = match measure {
                    Measure::Norm => 0,
                    Measure::AxisX => 1,
                    Measure::AxisY => 2,
                    Measure::AxisZ => 3,
                    Measure::PlaneY => 4,
                    Measure::AngleXY => 5,
                    Measure::AngleXZ => 6,
                    Measure::AngleYZ => 7,
                    Measure::LogNorm => 8,
                };
                let mode = match mode {
                    TrapMode::Minimum => 0,
                    TrapMode::Maximum => 1,
                    TrapMode::Last => 2,
                };
                (
                    Op::Trap,
                    self.field(id, "scale", 1),
                    slot.min(3) | (measure << 4) | (mode << 8),
                )
            }
            Iterate { body, .. } => {
                let p = self.fields(id, &[("iterations", 1), ("bailout", 1)]);
                let saved = regs.alloc();
                self.code
                    .push(Instruction::new(Op::LoopBegin, saved, dst, 0, 0, 0));
                let test = self.code.len();
                self.code
                    .push(Instruction::new(Op::LoopTest, dst, saved, 0, p, 0));
                for child in body {
                    self.point(child, dst, Some(saved), regs)?;
                }
                let mut end = Instruction::new(Op::LoopEnd, dst, saved, 0, 0, 0);
                end.jump = test as u32;
                self.code.push(end);
                self.code[test].jump = self.code.len() as u32;
                regs.release(saved);
                return Ok(());
            }
            k => bail!("node {id} is not a point operation: {k:?}"),
        };
        self.code
            .push(Instruction::new(op, dst, seed.unwrap_or(dst), 0, p, q));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_bundled_graphs_lower_with_bounded_live_registers() {
        for &(name, _) in syfrack_core::graph::bundled::BUNDLED {
            let g = Graph::bundled(name).unwrap();
            let p = Program::compile(&g).unwrap();
            assert!(!p.instructions.is_empty(), "{name}");
            assert!(p.registers < 128, "{name}: {}", p.registers);
            assert!(p.layout.pack(&g).iter().all(|v| v.is_finite()), "{name}");
            for i in &p.instructions {
                assert!(i.op < OPS.len() as u32);
                assert!(
                    i.dst < p.registers && i.a < p.registers && i.b < p.registers,
                    "{name}: {i:?}"
                );
                assert!(i.jump < p.instructions.len() as u32, "{name}: {i:?}");
            }
        }
    }
}
