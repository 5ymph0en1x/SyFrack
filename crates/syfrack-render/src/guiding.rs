//! Bounded, deterministic directional importance cache for surface transport.
//! Stratified path records and immutable spatial trees share the depth binding.
//! Each record has one GPU writer; updates only happen at sample boundaries.

pub const CELLS: usize = 256;
/// Bump when record layout, fitting policy or learning schedule changes.
pub const VERSION: u32 = 1;
pub const BINS: usize = 64; // 8 azimuth x 8 cosine-elevation, equal-area hemisphere
pub const ROW: usize = BINS + 1;
pub const RECORDS: usize = 8192;
pub const RECORD_ROW: usize = 8; // position, face/bin, proposal PDF, energy, valid, BSDF PDF
pub const BOUNDARIES: [u32; 5] = [16, 32, 64, 128, 256];
pub const BANK_WORDS: usize = RECORDS * RECORD_ROW * BOUNDARIES.len();
pub const HEADER_WORDS: usize = 9; // six roots, node/leaf/observation counts
pub const TREE_WORDS: usize = HEADER_WORDS + 2 * (2 * CELLS);
pub const PROPOSAL_OFFSET: usize = BANK_WORDS + TREE_WORDS;
pub const PROPOSAL_ROW: usize = 3 * BINS + 1;
pub const PROPOSAL_WORDS: usize = CELLS * PROPOSAL_ROW;
pub const BYTES: u64 = ((PROPOSAL_OFFSET + PROPOSAL_WORDS) * 4) as u64;
const LEAF_TAG: u32 = 0x80000000;
const MIN_CHILD: usize = 256;

/// Host-observed dispatch cost, including submission, waiting and validation.
/// These are not hardware timestamp-query measurements.
#[derive(Clone, Copy, Debug, Default)]
pub struct DispatchProfile {
    pub dispatches: u32,
    pub pixel_samples: u64,
    pub wall: std::time::Duration,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UpdateProfile {
    pub readback: std::time::Duration,
    pub decode: std::time::Duration,
    pub fit: std::time::Duration,
    pub upload: std::time::Duration,
    pub readback_bytes: u64,
    pub upload_bytes: u64,
}

/// Timings since this session was created/reset; checkpoints do not persist them.
#[derive(Clone, Copy, Debug, Default)]
pub struct GuidingProfile {
    /// Index is the number of committed training updates (0..=5).
    pub stages: [DispatchProfile; 6],
    /// BSDF, guided sampling, and training shader variants, respectively.
    pub variants: [DispatchProfile; 3],
    pub updates: [UpdateProfile; 5],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GuidingStats {
    pub updates: u32,
    /// Leaves with a fitted nonzero directional distribution.
    pub active_cells: usize,
    pub memory_bytes: u64,
    pub spatial_leaves: usize,
    pub training_records: usize,
    /// Fitted leaves accepted by held-out validation and actually queried.
    pub trusted_cells: usize,
}

pub(crate) fn flags(enabled: bool, updates: u32) -> u32 {
    if !enabled {
        0
    } else {
        u32::from(updates < BOUNDARIES.len() as u32) | (u32::from(updates > 0) << 1)
    }
}

/// First sample that can own a training slot in this round. Split dispatches
/// here so all other samples can use a shader without the training payload.
pub(crate) fn training_start(pixels: u64, end: u32) -> u32 {
    let stride = pixels.div_ceil(RECORDS as u64).max(1);
    let slots = pixels.div_ceil(stride);
    end.saturating_sub((RECORDS as u64).div_ceil(slots) as u32)
}

#[derive(Clone)]
struct Observation {
    position: [f32; 3],
    face: usize,
    bin: usize,
    energy: u32,
    pdf: f64,
    bsdf: f64,
    validation: Option<usize>,
}

struct Leaf {
    node: usize,
    samples: Vec<usize>,
    depth: usize,
}

pub(crate) struct Model {
    pub words: Vec<u32>,
    pub active: usize,
    pub leaves: usize,
    pub records: usize,
    pub trusted: usize,
}

/// Rebuild a density-adaptive median tree from the bounded, cumulative sample
/// pool. Normal faces have independent roots. No hash collision can join two
/// unrelated regions. Splits balance observations, not noisy energy outliers.
pub(crate) fn build_model(records: &[u32]) -> Model {
    assert!(records.len() <= BANK_WORDS && records.len() % RECORD_ROW == 0);
    let observations: Vec<_> = records
        .chunks_exact(RECORD_ROW)
        .enumerate()
        .filter_map(|(index, r)| {
            let position = [
                f32::from_bits(r[0]),
                f32::from_bits(r[1]),
                f32::from_bits(r[2]),
            ];
            let pdf = f32::from_bits(r[4]) as f64;
            let bsdf = f32::from_bits(r[7]) as f64;
            let h = (index as u32)
                .wrapping_mul(747796405)
                .wrapping_add(2891336453);
            let h = ((h >> ((h >> 28) + 4)) ^ h).wrapping_mul(277803737);
            let h = (h >> 22) ^ h;
            (r[6] == 1
                && (r[3] >> 8) < 6
                && (r[3] & 255) < BINS as u32
                && pdf.is_finite()
                && pdf > 0.0
                && bsdf.is_finite()
                && bsdf > 0.0
                && position.iter().all(|p| p.is_finite()))
            .then_some(Observation {
                position,
                face: (r[3] >> 8) as usize,
                bin: (r[3] & 255) as usize,
                energy: r[5],
                pdf,
                bsdf,
                validation: ((h & 3) == 0).then_some(((h >> 2) & 1) as usize),
            })
        })
        .collect();
    // Nodes are two words: leaf flag + cell, or left-child index + axis;
    // the second word holds the split position. Children are consecutive.
    let mut nodes = vec![[0u32; 2]; 6];
    let mut leaves: Vec<_> = (0..6)
        .map(|face| Leaf {
            node: face,
            depth: 0,
            samples: observations
                .iter()
                .enumerate()
                .filter_map(|(i, o)| (o.face == face).then_some(i))
                .collect(),
        })
        .collect();
    let mut blocked = vec![false; 6];
    let mut partitions: Vec<_> = leaves.iter().map(|l| l.samples.clone()).collect();
    while leaves.len() < CELLS {
        // Refine the most visited eligible region first, deterministically.
        let candidate = leaves
            .iter()
            .enumerate()
            .filter(|(i, l)| !blocked[*i] && l.samples.len() >= 2 * MIN_CHILD && l.depth < 15)
            .max_by_key(|(i, l)| (l.samples.len(), std::cmp::Reverse(*i)))
            .map(|(i, _)| i);
        let Some(i) = candidate else { break };
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for &s in &leaves[i].samples {
            for a in 0..3 {
                lo[a] = lo[a].min(observations[s].position[a]);
                hi[a] = hi[a].max(observations[s].position[a]);
            }
        }
        let mut axes = [0, 1, 2];
        axes.sort_by(|&a, &b| {
            (hi[b] as f64 - lo[b] as f64).total_cmp(&(hi[a] as f64 - lo[a] as f64))
        });
        let samples = &mut leaves[i].samples;
        let mut partition = None;
        for axis in axes {
            if hi[axis] == lo[axis] {
                continue;
            }
            samples.sort_unstable_by(|&a, &b| {
                observations[a].position[axis]
                    .total_cmp(&observations[b].position[axis])
                    .then(a.cmp(&b))
            });
            let median = observations[samples[samples.len() / 2]].position[axis];
            // Keep equal positions together, trying either edge of the group.
            let lower = samples.partition_point(|&s| observations[s].position[axis] < median);
            let upper = samples.partition_point(|&s| observations[s].position[axis] <= median);
            if let Some(split) = [lower, upper]
                .into_iter()
                .filter(|&s| s >= MIN_CHILD && samples.len() - s >= MIN_CHILD)
                .min_by_key(|&s| s.abs_diff(samples.len() / 2))
            {
                partition = Some((axis, split));
                break;
            }
        }
        let Some((axis, split)) = partition else {
            blocked[i] = true;
            continue;
        };
        let cut = observations[samples[split]].position[axis];
        let right = samples.split_off(split);
        let left_node = nodes.len();
        nodes[leaves[i].node] = [((left_node as u32) << 2) | axis as u32, cut.to_bits()];
        nodes.extend([[0; 2]; 2]);
        partitions.push(leaves[i].samples.clone());
        partitions.push(right.clone());
        leaves[i].node = left_node;
        leaves[i].depth += 1;
        leaves.push(Leaf {
            node: left_node + 1,
            samples: right,
            depth: leaves[i].depth,
        });
        blocked.push(false);
    }
    // Fit every candidate node so held-out data can choose a coarser field
    // when finer spatial partitions only increase statistical noise or cost.
    let mut histogram = vec![0.0f64; nodes.len() * ROW];
    for (cell, leaf) in leaves.iter().enumerate() {
        nodes[leaf.node][0] = LEAF_TAG | cell as u32;
    }
    for (node, samples) in partitions.iter().enumerate() {
        let row = &mut histogram[node * ROW..(node + 1) * ROW];
        for &s in samples {
            let o = &observations[s];
            if o.validation.is_none() {
                row[0] += 1.0;
                row[1 + o.bin] += (o.energy as f64).powi(2) * o.pdf;
            }
        }
        // RMS target accounts for noise in the later path's nested estimator,
        // rather than assuming that future scattering was sampled perfectly.
        for moment in &mut row[1..] {
            *moment = moment.sqrt();
        }
    }
    let (mut proposal, _) = build_proposal(&histogram);
    let mut selected_scores = vec![[0.0; 2]; nodes.len()];
    let mut trusted_subtree = vec![false; nodes.len()];
    for (node, samples) in partitions.iter().enumerate() {
        let row = &mut proposal[node * PROPOSAL_ROW..(node + 1) * PROPOSAL_ROW];
        row[0] = 0;
        let fitted = row[1] != 0;
        let mut scores = [[0.0; 5]; 2];
        let mut positive = [0; 2];
        for &s in samples {
            let o = &observations[s];
            let Some(group) = o.validation else { continue };
            if o.energy == 0 {
                continue;
            }
            positive[group] += 1;
            let guide_pdf = if fitted {
                f32::from_bits(row[1 + o.bin]) as f64 * BINS as f64 / (2.0 * std::f64::consts::PI)
            } else {
                o.bsdf
            };
            let moment = (o.energy as f64).powi(2) * o.pdf;
            for (i, score) in scores[group].iter_mut().enumerate() {
                let alpha = i as f64 / 8.0; // 0, 1/8, 1/4, 3/8, 1/2
                *score += moment / ((1.0 - alpha) * o.bsdf + alpha * guide_pdf);
            }
        }
        selected_scores[node] = [scores[0][0], scores[1][0]];
        // Fit and validation are disjoint. Require both held-out groups to
        // predict at least 5% improvement; uncertain leaves keep pure BSDF.
        if !fitted || positive.iter().any(|&n| n < 8) {
            continue;
        }
        let best = (1..5)
            .filter(|&i| scores.iter().all(|s| s[i] < s[0] * 0.95))
            .min_by(|&a, &b| {
                (scores[0][a] + scores[1][a]).total_cmp(&(scores[0][b] + scores[1][b]))
            });
        if let Some(i) = best {
            row[0] = (i as f32 / 4.0).to_bits(); // shader applies the 1/2 cap
            trusted_subtree[node] = true;
            selected_scores[node] = [scores[0][i], scores[1][i]];
        }
    }
    let mut collapsed = vec![true; nodes.len()];
    for node in (0..nodes.len()).rev() {
        if nodes[node][0] & LEAF_TAG != 0 {
            continue;
        }
        let left = (nodes[node][0] >> 2) as usize;
        let child_scores = [
            selected_scores[left][0] + selected_scores[left + 1][0],
            selected_scores[left][1] + selected_scores[left + 1][1],
        ];
        let child_trust = trusted_subtree[left] || trusted_subtree[left + 1];
        // Keep a split only if its validated children improve upon an accepted
        // parent by >5% in a held-out group. No evidence anywhere means BSDF.
        if child_trust
            && (!trusted_subtree[node]
                || (0..2).any(|g| child_scores[g] < selected_scores[node][g] * 0.95))
        {
            collapsed[node] = false;
            selected_scores[node] = child_scores;
            trusted_subtree[node] = child_trust;
        }
    }
    let mut compact_nodes = vec![[0u32; 2]; 6];
    let mut compact_proposal = Vec::new();
    let mut pending: Vec<_> = (0..6).rev().map(|root| (root, root)).collect();
    let mut active = 0;
    let mut trusted = 0;
    while let Some((source, dest)) = pending.pop() {
        if collapsed[source] {
            let cell = compact_proposal.len() / PROPOSAL_ROW;
            let row = &proposal[source * PROPOSAL_ROW..(source + 1) * PROPOSAL_ROW];
            active += usize::from(row[1] != 0);
            trusted += usize::from(row[0] != 0);
            compact_proposal.extend_from_slice(row);
            compact_nodes[dest] = [LEAF_TAG | cell as u32, 0];
        } else {
            let child = compact_nodes.len();
            compact_nodes.extend([[0; 2]; 2]);
            compact_nodes[dest] = [
                ((child as u32) << 2) | (nodes[source][0] & 3),
                nodes[source][1],
            ];
            let left = (nodes[source][0] >> 2) as usize;
            pending.extend([(left + 1, child + 1), (left, child)]);
        }
    }
    let leaf_count = compact_proposal.len() / PROPOSAL_ROW;
    compact_proposal.resize(PROPOSAL_WORDS, 0);
    let mut words = vec![0u32; TREE_WORDS + PROPOSAL_WORDS];
    for (root, out) in words[..6].iter_mut().enumerate() {
        *out = root as u32 | if trusted_subtree[root] { 0 } else { LEAF_TAG };
    }
    words[6] = compact_nodes.len() as u32;
    words[7] = leaf_count as u32;
    words[8] = observations.len() as u32;
    for (out, node) in words[HEADER_WORDS..].chunks_exact_mut(2).zip(compact_nodes) {
        out.copy_from_slice(&node);
    }
    words[TREE_WORDS..].copy_from_slice(&compact_proposal);
    Model {
        words,
        active,
        leaves: leaf_count,
        records: observations.len(),
        trusted,
    }
}

/// Normalize quantized energies with wide CPU sums; dark/sparse leaves use BSDF.
fn build_proposal(histogram: &[f64]) -> (Vec<u32>, usize) {
    assert_eq!(histogram.len() % ROW, 0);
    let mut proposal = vec![0u32; histogram.len() / ROW * PROPOSAL_ROW];
    let mut active = 0;
    for (src, dst) in histogram
        .chunks_exact(ROW)
        .zip(proposal.chunks_exact_mut(PROPOSAL_ROW))
    {
        let total: f64 = src[1..].iter().map(|&v| v as f64).sum();
        if src[0] <= 32.0 || total == 0.0 {
            continue;
        }
        // The cosine prior keeps
        // unexplored bins supported without spending rays below the surface.
        active += 1;
        let mut scaled = [0.0; BINS];
        let mut small = Vec::with_capacity(BINS);
        let mut large = Vec::with_capacity(BINS);
        for (bin, weight) in src[1..].iter().enumerate() {
            let row = (bin / 8) as f64;
            let cosine_mass = (2.0 * row + 1.0) / 512.0;
            scaled[bin] =
                ((*weight as f64 / total) * (15.0 / 16.0) + cosine_mass / 16.0) * BINS as f64;
            if scaled[bin] < 1.0 {
                small.push(bin);
            } else {
                large.push(bin);
            }
            dst[1 + BINS + bin] = 1.0f32.to_bits();
            dst[1 + 2 * BINS + bin] = bin as u32;
        }
        // Vose's deterministic O(n) construction, O(1) GPU selection.
        while !small.is_empty() && !large.is_empty() {
            let s = small.pop().unwrap();
            let l = large.pop().unwrap();
            dst[1 + BINS + s] = (scaled[s].clamp(0.0, 1.0) as f32).to_bits();
            dst[1 + 2 * BINS + s] = l as u32;
            scaled[l] -= 1.0 - scaled[s];
            if scaled[l] < 1.0 {
                small.push(l);
            } else {
                large.push(l);
            }
        }
        // Evaluate the distribution *actually* produced by the uploaded f32
        // cutoffs, rather than pretending the desired f64 masses are exact.
        let mut mass = [0.0f64; BINS];
        for bin in 0..BINS {
            let cut = f32::from_bits(dst[1 + BINS + bin]) as f64;
            mass[bin] += cut / BINS as f64;
            mass[dst[1 + 2 * BINS + bin] as usize] += (1.0 - cut) / BINS as f64;
        }
        for (out, mass) in dst[1..1 + BINS].iter_mut().zip(mass) {
            *out = (mass as f32).to_bits();
        }
    }
    (proposal, active)
}

pub(crate) fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn training_dispatch_window_covers_every_possible_record_owner() {
        for pixels in [1u64, 17, 1024, 8192, 10_240, 19_200, 8_294_400] {
            let stride = pixels.div_ceil(RECORDS as u64).max(1);
            let slots = pixels.div_ceil(stride);
            for end in BOUNDARIES {
                let start = training_start(pixels, end);
                let capacity = (RECORDS as u64).div_ceil(slots);
                for sample in 0..end {
                    let owns_slot = ((end - 1 - sample) as u64) < capacity;
                    assert_eq!(
                        sample >= start,
                        owns_slot,
                        "pixels {pixels}, end {end}, sample {sample}"
                    );
                }
            }
        }
    }

    #[test]
    fn alias_pdf_matches_uploaded_sampling_with_full_support_and_dark_fallback() {
        let mut histogram = vec![0.0f64; CELLS * ROW];
        histogram[0] = 100.0;
        histogram[3] = u32::MAX as f64 * 40960.0;
        histogram[ROW] = 100.0; // well-visited black region
        histogram[2 * ROW] = 31.0; // insufficient observations
        histogram[2 * ROW + 1] = 1_000_000.0;
        let (proposal, active) = build_proposal(&histogram);
        assert_eq!(active, 1);
        let mut mass = [0.0f64; BINS];
        for bin in 0..BINS {
            let cut = f32::from_bits(proposal[1 + BINS + bin]) as f64;
            assert!((0.0..=1.0).contains(&cut));
            let alias = proposal[1 + 2 * BINS + bin] as usize;
            assert!(alias < BINS);
            mass[bin] += cut / BINS as f64;
            mass[alias] += (1.0 - cut) / BINS as f64;
        }
        assert!((mass.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        for (bin, actual) in mass.iter().enumerate() {
            let pdf_mass = f32::from_bits(proposal[1 + bin]) as f64;
            assert!(pdf_mass > 0.0);
            assert!((pdf_mass - actual).abs() < 3e-8);
        }
        assert!(proposal[PROPOSAL_ROW..].iter().all(|&v| v == 0));
    }

    #[test]
    fn shader_cache_dimensions_match_host_and_fit_minimum_storage_limits() {
        let source = crate::shader_source::GUIDING;
        assert!(source.contains(&format!("const GUIDE_CELLS: u32 = {CELLS}u;")));
        assert!(source.contains(&format!("const GUIDE_BINS: u32 = {BINS}u;")));
        assert!(BYTES < 2 * 1024 * 1024);
        let mut cfg = crate::RenderConfig::new(3840, 2160, 256);
        cfg.guiding = true;
        cfg.validate(&wgpu::Limits::default())
            .expect("4K guiding fits the minimum binding limit");
    }

    fn record(position: [f32; 3], face: u32, bin: u32) -> [u32; RECORD_ROW] {
        [
            position[0].to_bits(),
            position[1].to_bits(),
            position[2].to_bits(),
            (face << 8) | bin,
            0.1f32.to_bits(),
            4096,
            1,
            0.1f32.to_bits(),
        ]
    }

    fn lookup(model: &Model, p: [f32; 3], face: usize) -> usize {
        let mut node = (model.words[face] & !LEAF_TAG) as usize;
        for _ in 0..16 {
            let offset = HEADER_WORDS + node * 2;
            let tag = model.words[offset];
            if tag & LEAF_TAG != 0 {
                return (tag & !LEAF_TAG) as usize;
            }
            let cut = f32::from_bits(model.words[offset + 1]);
            node = (tag >> 2) as usize + usize::from(p[(tag & 3) as usize] >= cut);
        }
        panic!("tree exceeds GPU traversal budget")
    }

    #[test]
    fn adaptive_tree_separates_cavities_and_opposing_normals_without_hash_aliases() {
        let mut records = Vec::new();
        for (x, face, bin) in [(-10.0, 0, 3), (10.0, 0, 42), (-10.0, 1, 17)] {
            for _ in 0..512 {
                records.extend(record([x, 0.0, 0.0], face, bin));
            }
        }
        let a = build_model(&records);
        let b = build_model(&records);
        assert_eq!(a.words, b.words);
        let left = lookup(&a, [-10.0, 0.0, 0.0], 0);
        let right = lookup(&a, [10.0, 0.0, 0.0], 0);
        let back = lookup(&a, [-10.0, 0.0, 0.0], 1);
        assert_ne!(left, right);
        assert_ne!(left, back);
        assert_eq!(a.active, 3);
        assert_eq!(a.trusted, 3);
        assert_eq!(a.records, 1536);
        for (cell, peak) in [(left, 3), (right, 42), (back, 17)] {
            let row = &a.words[TREE_WORDS + cell * PROPOSAL_ROW..];
            assert!(f32::from_bits(row[1 + peak]) > 0.93);
        }
    }

    #[test]
    fn dense_and_degenerate_observations_respect_memory_and_depth_budgets() {
        let mut records = Vec::with_capacity(BANK_WORDS);
        for i in 0..RECORDS * BOUNDARIES.len() {
            records.extend(record([i as f32, 0.0, 0.0], 4, (i / 4096) as u32));
        }
        let a = build_model(&records);
        assert!(a.leaves > 6 && a.leaves <= CELLS);
        assert!(a.words[6] <= (2 * CELLS) as u32);
        assert_eq!(a.words.len() * 4 + BANK_WORDS * 4, BYTES as usize);
        for i in [0, 512, 8191, 40959] {
            assert!(lookup(&a, [i as f32, 0.0, 0.0], 4) < CELLS);
        }
        let identical: Vec<_> = (0..1024).flat_map(|_| record([0.0; 3], 0, 0)).collect();
        let b = build_model(&identical);
        assert_eq!(b.leaves, 6);
        assert_eq!(b.active, 1);
        assert_eq!(build_model(&[]).active, 0);
        let outlier: Vec<_> = (0..2048)
            .flat_map(|i| {
                record(
                    [if i == 0 { 100000.0 } else { 0.0 }, i as f32, 0.0],
                    0,
                    if i < 1024 { 0 } else { 42 },
                )
            })
            .collect();
        assert!(
            build_model(&outlier).leaves > 6,
            "one outlier must not block other useful split axes"
        );
    }

    #[test]
    fn held_out_validation_keeps_an_already_optimal_cosine_proposal() {
        let mut records = Vec::new();
        for bin in 0..BINS {
            let row = bin / 8;
            let bsdf = ((row as f32 + 0.5) / 8.0) / std::f32::consts::PI;
            for _ in 0..8 * (2 * row + 1) {
                let mut r = record([0.0; 3], 0, bin as u32);
                r[4] = bsdf.to_bits();
                r[7] = bsdf.to_bits();
                records.extend(r);
            }
        }
        let model = build_model(&records);
        assert_eq!(model.active, 1);
        assert_eq!(
            model.trusted, 0,
            "no predicted improvement over optimal cosine"
        );
    }
}
