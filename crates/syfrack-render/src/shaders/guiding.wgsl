// Bounded path records, an adaptive tree and frozen directional proposals.
// Depth leaves room for this cache even at 4K with a 128 MiB binding limit.
// A pixel/record has exactly one writer. The tree/proposal region is uploaded
// between completed dispatches and stays immutable while shaders read it.
// Ordinary storage access is sufficient; no cross-invocation atomic is needed.
@group(0) @binding(2) var<storage, read_write> depth_buf: array<u32>;
const GUIDE_CELLS: u32 = 256u;
const GUIDE_BINS: u32 = 64u;
const GUIDE_RECORDS: u32 = 8192u;
const GUIDE_RECORD_ROW: u32 = 8u;
const GUIDE_BANK: u32 = 5u * GUIDE_RECORDS * GUIDE_RECORD_ROW;
const GUIDE_HEADER: u32 = 9u;
const GUIDE_TREE_WORDS: u32 = GUIDE_HEADER + 4u * GUIDE_CELLS;
const GUIDE_PROPOSAL: u32 = GUIDE_BANK + GUIDE_TREE_WORDS;
const GUIDE_PROPOSAL_ROW: u32 = 3u * GUIDE_BINS + 1u;
const NO_GUIDE_RECORD: u32 = 0xffffffffu;
// Pipeline constants remove unused learning/sampling code and its live path
// payload from ordinary samples. Runtime flags still gate learned confidence.
override GUIDE_TRAINING: bool = true;
override GUIDE_SAMPLING: bool = true;

fn guide_base() -> u32 { return u.image_size.x * u.image_size.y; }

fn guide_face(n: vec3f) -> u32 {
    let an = abs(n);
    var axis = 2u;
    if (an.x >= max(an.y, an.z)) { axis = 0u; }
    else if (an.y >= an.z) { axis = 1u; }
    return 2u * axis + select(0u, 1u, n[axis] < 0.0);
}

fn guide_cell(p: vec3f, n: vec3f) -> u32 {
    let base = guide_base() + GUIDE_BANK;
    var node = depth_buf[base + guide_face(n)];
    if ((node & 0x80000000u) != 0u) { return NO_GUIDE_RECORD; }
    for (var level = 0u; level < 16u; level++) {
        let offset = base + GUIDE_HEADER + 2u * node;
        let tag = depth_buf[offset];
        if ((tag & 0x80000000u) != 0u) { return tag & 0x7fffffffu; }
        let cut = bitcast<f32>(depth_buf[offset + 1u]);
        node = (tag >> 2u) + select(1u, 0u, p[tag & 3u] < cut);
    }
    return 0u;
}

// Space-stratified pixels on large images; on small images, fill the pool
// with the last few samples of this round. Each slot has exactly one owner,
// independently of tile order and samples grouped into a dispatch.
fn guide_record_slot(pixel: u32, sample_index: u32) -> u32 {
    if (!GUIDE_TRAINING) { return NO_GUIDE_RECORD; }
    if ((u.guiding_flags & 1u) == 0u || sample_index >= u.guiding_training_end) { return NO_GUIDE_RECORD; }
    let pixels = u.image_size.x * u.image_size.y;
    let stride = max((pixels + GUIDE_RECORDS - 1u) / GUIDE_RECORDS, 1u);
    if (pixel % stride != 0u) { return NO_GUIDE_RECORD; }
    let slots = (pixels + stride - 1u) / stride;
    let age = u.guiding_training_end - 1u - sample_index;
    if (age >= (GUIDE_RECORDS + slots - 1u) / slots) { return NO_GUIDE_RECORD; }
    let slot = age * slots + pixel / stride;
    if (slot >= GUIDE_RECORDS) { return NO_GUIDE_RECORD; }
    return u.guiding_record_round * GUIDE_RECORDS + slot;
}

fn guide_bin(d: vec3f) -> u32 {
    let phi = atan2(d.y, d.x);
    let azimuth = (phi + 2.0 * PI) % (2.0 * PI);
    let x = min(u32(azimuth * (8.0 / (2.0 * PI))), 7u);
    let y = min(u32(clamp(d.z, 0.0, 1.0) * 8.0), 7u);
    return y * 8u + x;
}

fn guide_strength(cell: u32, s: Surface, p_spec: f32) -> f32 {
    if (!GUIDE_SAMPLING) { return 0.0; }
    if ((u.guiding_flags & 2u) == 0u || cell == NO_GUIDE_RECORD) { return 0.0; }
    let confidence = bitcast<f32>(depth_buf[guide_base() + GUIDE_PROPOSAL + cell * GUIDE_PROPOSAL_ROW]);
    if (confidence == 0.0) { return 0.0; }
    // Sharp GGX reflection keeps its VNDF proposal; diffuse and rough metal
    // can use the learned product of incident light and local scattering.
    return 0.5 * confidence * max(1.0 - p_spec, min(4.0 * s.alpha, 1.0));
}

fn guide_pdf(cell: u32, n: vec3f, d: vec3f) -> f32 {
    let local = transpose(tangent_frame(n)) * d;
    if (local.z <= 0.0) { return 0.0; }
    let bin = guide_bin(local);
    let mass = bitcast<f32>(depth_buf[guide_base() + GUIDE_PROPOSAL + cell * GUIDE_PROPOSAL_ROW + 1u + bin]);
    return mass * (f32(GUIDE_BINS) / (2.0 * PI));
}

fn continuation_pdf(s: Surface, geometric_n: vec3f, v: vec3f, d: vec3f, p_spec: f32, cell: u32, strength: f32) -> f32 {
    let bsdf = bsdf_pdf(s, v, d, p_spec);
    if (strength == 0.0) { return (1.0 - s.transmission_pdf) * bsdf; }
    return (1.0 - s.transmission_pdf) * ((1.0 - strength) * bsdf + strength * guide_pdf(cell, geometric_n, d));
}

fn guide_sample(cell: u32, n: vec3f, choose: f32, r1: f32, r2: f32) -> vec3f {
    let column_f = choose * f32(GUIDE_BINS);
    let column = min(u32(column_f), GUIDE_BINS - 1u);
    let base = guide_base() + GUIDE_PROPOSAL + cell * GUIDE_PROPOSAL_ROW + 1u;
    let cut = bitcast<f32>(depth_buf[base + GUIDE_BINS + column]);
    var bin = column;
    if (column_f - f32(column) >= cut) {
        bin = depth_buf[base + 2u * GUIDE_BINS + column];
    }
    let phi = 2.0 * PI * (f32(bin % 8u) + r1) / 8.0;
    let z = (f32(bin / 8u) + r2) / 8.0;
    let r = sqrt(max(0.0, 1.0 - z * z));
    return tangent_frame(n) * vec3f(r * cos(phi), r * sin(phi), z);
}

struct GuideRecord {
    valid: bool,
    throughput: vec3f,
    baseline: vec3f,
}

// Write spatial metadata at the selected vertex, rather than keeping its
// position/face/direction live in registers through the rest of the path.
fn guide_capture(slot: u32, p: vec3f, n: vec3f, d: vec3f, pdf: f32, bsdf: f32) {
    let base = guide_base() + slot * GUIDE_RECORD_ROW;
    depth_buf[base] = bitcast<u32>(p.x);
    depth_buf[base + 1u] = bitcast<u32>(p.y);
    depth_buf[base + 2u] = bitcast<u32>(p.z);
    depth_buf[base + 3u] = (guide_face(n) << 8u) | guide_bin(transpose(tangent_frame(n)) * d);
    depth_buf[base + 4u] = bitcast<u32>(pdf);
    depth_buf[base + 7u] = bitcast<u32>(bsdf);
}

fn guide_learn(slot: u32, record: GuideRecord, radiance: vec3f) {
    if (slot == NO_GUIDE_RECORD || !record.valid) { return; }
    // Back-propagate all later surface, emitter and volume contributions.
    // This is an estimate of BSDF*cos*Li / continuation_pdf in this bin.
    let rgb = max(radiance - record.baseline, vec3f(0.0)) / max(record.throughput, vec3f(1e-20));
    let weight = dot(rgb, vec3f(0.2126, 0.7152, 0.0722));
    let base = guide_base() + slot * GUIDE_RECORD_ROW;
    depth_buf[base + 6u] = 1u;
    if (weight == weight && weight > 0.0) {
        // Clamp only the *training proposal*, never image radiance.
        depth_buf[base + 5u] = u32(min(weight, 65535.0) * 4096.0);
    }
}
