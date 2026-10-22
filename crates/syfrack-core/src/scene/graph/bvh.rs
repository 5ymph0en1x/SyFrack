//! Immutable, stackless BVH of the *fences* of independently transformed copies.
//! A fence is part of an Instances node's distance field, even if its template
//! extends outside it. Its signed distance is therefore a valid lower bound.
use glam::{EulerRot, Quat, Vec3};

use super::Placement;

#[derive(Clone, Copy, Debug)]
pub struct InstanceBounds {
    pub center: Vec3,
    pub half: Vec3,
    /// First node following this subtree (preorder layout).
    pub escape: usize,
    /// Original placement index; None for an interior node.
    pub placement: Option<usize>,
}

impl InstanceBounds {
    pub fn distance(&self, point: Vec3) -> f32 {
        let q = (point - self.center).abs() - self.half;
        q.max(Vec3::ZERO).length() + q.max_element().min(0.0)
    }
}

/// Construct once per resource update, never per ray. Stable median splits and
/// original placement indices preserve the graph's material tie-breaking rule.
pub fn build(placements: &[Placement], half: Vec3) -> Vec<InstanceBounds> {
    if placements.is_empty() {
        return Vec::new();
    }
    let mut leaves: Vec<_> = placements
        .iter()
        .enumerate()
        .map(|(index, p)| {
            let a = p.angles * (std::f32::consts::PI / 180.0);
            let r = Quat::from_euler(EulerRot::XYZ, a.x, a.y, a.z);
            let h = half * p.scale;
            let extent =
                (r * Vec3::X).abs() * h.x + (r * Vec3::Y).abs() * h.y + (r * Vec3::Z).abs() * h.z;
            // Cover both affine roundoff and cancellation at large coordinates.
            let pad = (p.position.abs() + extent) * (16.0 * f32::EPSILON) + Vec3::splat(0.002);
            InstanceBounds {
                center: p.position,
                half: extent + pad,
                escape: 0,
                placement: Some(index),
            }
        })
        .collect();
    fn split(leaves: &mut [InstanceBounds], nodes: &mut Vec<InstanceBounds>) {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for leaf in leaves.iter() {
            lo = lo.min(leaf.center - leaf.half);
            hi = hi.max(leaf.center + leaf.half);
        }
        let index = nodes.len();
        let center = lo * 0.5 + hi * 0.5;
        let half = (hi - center).max(center - lo) + Vec3::splat(0.002);
        nodes.push(InstanceBounds {
            center,
            half,
            escape: 0,
            placement: None,
        });
        if leaves.len() == 1 {
            nodes[index].placement = leaves[0].placement;
        } else {
            let extent = hi - lo;
            let axis = if extent.x >= extent.y && extent.x >= extent.z {
                0
            } else if extent.y >= extent.z {
                1
            } else {
                2
            };
            let middle = leaves.len() / 2;
            leaves.select_nth_unstable_by(middle, |a, b| {
                a.center[axis]
                    .total_cmp(&b.center[axis])
                    .then(a.placement.cmp(&b.placement))
            });
            let (left, right) = leaves.split_at_mut(middle);
            split(left, nodes);
            split(right, nodes);
        }
        nodes[index].escape = nodes.len();
    }
    let mut nodes = Vec::with_capacity(2 * leaves.len() - 1);
    split(&mut leaves, &mut nodes);
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hierarchy_contains_rotated_scaled_fences_and_visits_every_leaf_once() {
        let placements: Vec<_> = (0..1031)
            .map(|i| Placement {
                position: Vec3::new((i % 31) as f32 * 3.0, (i / 31) as f32, i as f32 * 0.01),
                angles: Vec3::new(i as f32, 31.0, -19.0),
                scale: Vec3::new(0.2, 2.0, 1.3),
            })
            .collect();
        let half = Vec3::new(1.0, 0.3, 2.0);
        let nodes = build(&placements, half);
        assert_eq!(nodes.len(), 2 * placements.len() - 1);
        let mut seen = vec![false; placements.len()];
        for (i, n) in nodes.iter().enumerate() {
            assert!(n.escape > i && n.escape <= nodes.len());
            if let Some(p) = n.placement {
                assert!(!seen[p]);
                seen[p] = true;
                let a = placements[p].angles * (std::f32::consts::PI / 180.0);
                let r = Quat::from_euler(EulerRot::XYZ, a.x, a.y, a.z);
                for x in [-1.0, 1.0] {
                    for y in [-1.0, 1.0] {
                        for z in [-1.0, 1.0] {
                            let corner = placements[p].position
                                + r * (half * placements[p].scale * Vec3::new(x, y, z));
                            assert!(n.distance(corner) < 0.0);
                            for parent in &nodes[..i] {
                                if parent.escape > i {
                                    assert!(parent.distance(corner) < 0.0);
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(seen.into_iter().all(|v| v));
        assert!(nodes[0].distance(Vec3::splat(1e5)) > 1.0);
    }
}
