// SyFrack distance-field probe; prepended with common.wgsl and the generated
// de_sample at build time.
// Evaluates de_sample at each input point for the CPU/GPU consistency test.

@group(0) @binding(1) var<storage, read> points: array<vec4f>;
@group(0) @binding(2) var<storage, read_write> results: array<vec4f>;
@group(0) @binding(3) var<storage, read_write> results_traps: array<vec4f>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let i = id.x;
    if (i >= arrayLength(&points)) {
        return;
    }
    let s = de_sample_exact(points[i].xyz);
    results[i] = vec4f(s.distance, s.traps.x, f32(s.iterations), f32(s.material));
    results_traps[i] = s.traps;
}
