# Hybrid performance, quality and repeatability

The production direction is now GPU-only rendering. `auto` no longer starts a
software CPU worker; the native hybrid engine and reports below remain explicit
diagnostics and historical measurements. The 128-sample Orfevrerie hybrid
qualification was stopped at the user's request and has no result.
Portable CPU/GPU bit equality remains unqualified.

## Measure the cumulative contribution

```powershell
.\scripts\with-vulkan-cpu.ps1 -Executable .\target\debug\syfrack.exe -RenderArgs @('benchmark-hybrid', 'scenes/mandelbulb.ron', '--out', 'target/hybrid.ron', '--repeats', '3')
```

The benchmark warms both pipelines, then alternates GPU-only, CPU-only and
simultaneous hybrid exports in repeated, interleaved runs. Its speedup uses
median end-to-end times, including slot creation, scheduling and readback,
excluding encoding and cold compilation. Reports retain every run, producer,
tile count and each processor's last-tile delivery time. The ideal throughput
sum is an estimate from isolated rates, not a performance guarantee: contention
and different tile costs can prevent reaching it. Cold CPU preparation runs
in the background during ordinary hybrid exports; it claims no real tile until
the driver has been warmed. A short export can finish before that preparation.

Version-3 benchmark reports measure the intersection of the CPU and GPU
submission-to-completion intervals, excluding warm-up and tile readback. The
command rejects a hybrid performance claim if either processor contributes no
tile or those intervals never overlap. These are host-clock in-flight intervals,
not hardware occupancy measurements. Console phase markers identify each
warm-up and timed baseline explicitly. See the [local performance study](benchmarks/hybrid-20261018/README.md)
for the measured gain and its limits on the Quadro target.

Quality is compared in scene-linear HDR against the GPU-only image. The default
hybrid acceptance criterion is RGB RMSE <= 1e-4 + 0.01 * reference RMS, with an
independent alpha check and rejection of all nonfinite values. Reports include
the relative RMSE, maximum error and worst pixel; a small global RMSE can still
hide a local outlier. They retain the worst hybrid RMSE across repetitions.
The full CPU image has separate diagnostic metrics and a separate tolerance
result; its error is not the assembled hybrid image's error. The command saves
the report before returning a failure for an out-of-tolerance hybrid image.
These are measured scene/device combinations, not universal error bounds.

The fast hybrid renderer uses native floating point on two different adapters.
It is **not qualified for portable bit-exact output**. A tile has one owner and
its samples accumulate sequentially, but this preserves summation order only;
it cannot make different per-sample arithmetic results identical.

## Executable phase-1 gate

```powershell
cargo run --locked -p syfrack-cli -- parity --points 100000 --out target/parity.ron
```

This command saves its RON report before returning a failure when the strict
gate fails. Use `--report-only` to collect diagnostics without asserting parity.
With the Vulkan CPU driver launcher, `--cpu` audits software WGSL against the
Rust evaluator. The integration-test equivalent is:

```text
cargo test --locked -p syfrack-render --test gpu_parity -- --ignored --nocapture
```

The gate requires all eleven bundled graphs, at least 100,000 input points per
graph and all eight arithmetic probes. Graph probes compare the raw bits of
distance and all four traps, then exact iteration/material IDs. They include
origin, poles and signed zeros, and reject nonfinite distances even when the
two bit patterns coincide. Reports retain the first mismatch, ULP/absolute
errors, adapter and driver. The host generates each point once and uploads the
same binary32 values. The distance-only shader avoids compiling the path tracer.

The arithmetic probe separately measures the candidate `sin`, `cos`, `atan2`,
`log2`, `exp2`, `pow`, native `fma` and native `sqrt`. `det_math.rs` generates
exact hexadecimal WGSL coefficients from its one Rust coefficient table.
`det_math.wgsl` uses Cody-Waite reduction and Taylor/Horner polynomials with
explicit FMA. These are experimental candidates, not Remez approximations or
certified replacements. Their scalar approximation errors are tested against
native references, separately from the strict CPU/GPU bit comparison. Domains
are explicit: trigonometry is limited to |x| <= 8192, logarithms to positive
normal inputs, and exponentiation to exponent [-126,128). They are **not injected
into production graph or transport shaders** while the arithmetic gate fails.

The initial native audit on the Quadro K3100M/Vulkan on 2026-10-18 found distance
bit mismatches on all eleven graphs. For example, sphere had 22,817 mismatches
and Mandelbox 58,344 out of 100,000 points each. Native evaluator differences
also include quaternion versus sequential Euler rotation and integer-power
specialization choices, beyond SFU accuracy. Replacing transcendentals alone
does not resolve the complete evaluator contract.

A passing finite corpus would qualify that measured combination for that
experiment; it would not prove equality over every input, shader or adapter.

## Why the proposed FMA protocol is insufficient

The [WGSL floating-point accuracy rules](https://www.w3.org/TR/WGSL/#floating-point-accuracy)
allow `fma` to inherit the accuracy of `x*y+z`, and `sqrt` that of
`1/inverseSqrt(x)`. The language does not require the assumed IEEE rounding
mode and permits reassociation, fusion and subnormal flushing. SIMD on the CPU
does not strengthen the GPU contract. Explicit FMA and matching coefficients
are therefore experiments to measure, not a portable proof.

The complete transport also uses floating point in camera rays, normalization,
BRDF/MIS, free-flight sampling, texture filtering and auxiliary moments. A
distance-only certificate is a prerequisite, not a full path-tracer certificate.
Exact accumulation of already-different samples cannot fix their disagreement.

## Safe changes applied independently

The immutable tile queue uses a single atomic pair of front/back cursors. GPU
workers claim from the front and the CPU worker from the back. One CAS owns the
last tile, preventing the race caused by two independent cursors. Whole-tile
ownership and bounded device/readback pools remain intact. No fixed 80/20
partition, partial-sum merging or transfer of live accumulators is introduced.
Measured time per pixel-sample makes a slower CPU retire before claiming a
tail tile that is likely to outlive the remaining GPU work. Unclaimed tiles
remain available to the GPU, and the faster worker keeps consuming the queue.

New version-2 tiled checkpoints record adapter/driver, workgroup dimensions and the actual
shader implementation, including CPU specialization. Resume refuses a changed
implementation before starting workers. Version 2 requires producer metadata and
is rejected by older binaries that lack this check. Older version-1 states remain readable
but lack this provenance. Matching provenance does not certify interchangeable
native CPU/GPU arithmetic or deterministic hybrid tile assignment.
An explicit `syfrack resume ... --backend hybrid|tiled|cpu` accepts native
backend differences, keeps completed tile values and records prior producers.
It does not promise identical output hashes after changing backend. Deferred
CPU preparation verifies its saved implementation before claiming any work.

For experiments that need repeatable files on the same setup, select
`--backend tiled`, retain the same build/device/driver, and resume the checkpoint
with that setup. The checkpoint integration test compares both the raw HDR and
the **complete EXR and PNG file bytes** after pause/resume, including global
denoising, auto-exposure, grain and dithering. Cross-device hashes remain unqualified.
EXR compression remains parallel, with an explicitly increasing block order so
thread completion order does not change file bytes from identical HDR pixels.

## Optional portable bit-exact research

1. Keep the strict gate closed until arithmetic and all graph semantics agree.
   A portable strict path would need software binary32 operations implemented
   with integer operations, or explicitly required hardware float controls;
   it must cover rounding, subnormals, division, square root and exceptional
   values as well as transcendentals. Measure its accuracy and cost first.
2. Generate scalar Rust and WGSL from one canonical operation contract, then
   qualify a complete scalar path tracer, every auxiliary plane and asset
   preparation. Native CPU SIMD tracing is not enabled before these gates pass.
3. Add packet SIMD with runtime CPU feature detection and scalar fallback;
   preserve per-lane operation order and validate every packet mask. Measure
   cold compilation, warm rendering and end-of-render latency before claiming
   a speedup or increasing GPU batch sizes.
4. Qualify raw-accumulator migration and every resume backend change separately.
   Retain whole-tile ownership until that protocol is proven. A coarse primary
   depth prepass is unsafe as `t_min`: cone bounds must cover jitter, aperture,
   silhouettes and thin geometry before it may skip any ray segment.

These gates concern a future portable bit-exact CPU packet tracer. The production
hybrid engine instead uses the shared WGSL transport with measured native errors.
