# Architecture

SyFrack is a Cargo workspace with six crates. Studio and the CLI share the same
scene model, validation, renderer and colour pipeline.

```text
Studio / CLI
    │
    ├── syfrack-core: RON scenes, graph, parameter bounds, CPU evaluation
    ├── syfrack-render: wgpu, generated WGSL, sampling, sessions/checkpoints
    ├── syfrack-color: linear HDR development, PNG/EXR encoding
    └── syfrack-i18n: compiled messages and language selection
```

## Scene and graph

`syfrack-core` owns the serializable scene contract. Graph nodes describe spatial
transforms, recursive operations, traps, primitives and combinations. Validation
rejects invalid links, cycles and out-of-bounds values. CPU evaluation is useful
for parity checks against generated GPU code.

The optional Forme source language compiles into this graph. New source exports
use English operation names. The parser also accepts the legacy French
vocabulary; RON enum names and parameter paths are language-independent.

## Rendering

Studio's Simple and Advanced interfaces are presentations of the same
`AppState` document. The local `InterfaceMode` preference is independent of
scene serialization and undo/redo. Simple edits authoritative `creation.nodes`
parameters and connected palettes; it does not maintain a second scene model.
Switching to Simple from the graph returns to the workshop without losing the
graph selection. Incomplete graph drafts remain available in Advanced.

The final-render worker publishes its complete PNG source image before
`Event::Finished`, including optional denoising. Periodic previews alone cannot
represent the last completed batch; both native windows share egui's painter
and receive the terminal texture update.

Studio's default 3D preview uses `syfrack-render::control`, a fixed GPU interpreter.
Graph edits lower to typed instructions and parameter buffers; they never create
a graph-specific shader. Last-consumer register reuse limits scratch space, and
instance templates have separate register banks and an explicit GPU call stack.
Each dispatch executes at most 128 instructions per pixel, preserving program
counters, loop seeds/counters, registers, calls and ray-marching state in SoA
storage. Nested iterations retain their full counts. Instances use the same
packed inverse transforms and stackless BVH as the production graph evaluator.
The interpreter evaluates the exact distance field rather than treating a
conservative acceleration envelope as a surface.

The two control entry points are compiled once per renderer/device and shared
by its clones. A 16-byte asynchronous status readback gates further work; the
interactive tick never waits for GPU completion. Scratch is capped at 64 MiB
and device buffer limits. Larger programs reduce control image resolution,
preserving graph operations and iterations. Surface normals and depth become
available only after an accepted field intersection. Rays have no artificial tracing-step
cutoff: they resume until a hit or exit from the rendering domain. Non-finite
fields are reported as graph evaluation errors naming their source node, never
silently converted into empty space.
The control view shows the graph with a pinhole camera and neutral lighting;
it does not simulate scene materials, fog, ocean or lens blur.

**Compile specialized preview** opts into the existing native path tracer.
Control remains visible while compilation runs. One active request and one
coalesced desired scene prevent compilation storms; an obsolete result cannot
replace the current graph. Adoption holds the compiled pipeline directly, so an
LRU eviction cannot trigger another synchronous shader compilation. Changing topology returns to control without starting
another native compile. Incomplete graphs retain the last valid image, marked
as stale. Control and native accumulations are separate, and exports continue
to use the production renderer and checkpoint format.

`syfrack-render` provides progressive render sessions. 3D uses GPU tracing of
distance-estimated geometry and stochastic RGB light transport. The analytic
2D canvas and flame accumulators have their own sampling/reconstruction paths.
Volume and ocean shaders add specialised approximations.

Shader topology and scalar values are treated separately where possible.
Power 6/8 has a specialised polynomial implementation. Changing a display
language must never alter shader fingerprints, scene values or sampling.

GPU work is tiled and scheduled adaptively. Preview latency and final export
latency have different budgets. Session checkpoints preserve the actual samples,
tile state and scene. They are not a cross-version interchange format.

Final renders in Studio and the CLI use `ExportSession`. `--backend tiled`
keeps two reusable sets of compact tile buffers and staging buffers on the GPU.
The work queue contains clipped CPU jobs of at most 16×16 pixels, ordered inside
64×64 blocks for small images and 128×128 blocks from 65,536 pixels upward.
The GPU atomically claims complete rectangular groups of these
jobs for a larger dispatch; the CPU takes one small job at a time. Checkpoint
holes and concurrent CPU claims cannot be included in a GPU group a second time.
An immutable tile array and one atomic pair of cursors let GPU and CPU claim
opposite ends without a mutex or a last-tile race. One worker owns a tile through
its final sample; completed sample sums are never merged across workers.
Measured per-device time per pixel-sample prevents a slower CPU from claiming
the last jobs when those would probably finish after the remaining GPU queue.
Camera rays, sampling indices and random seeds use full-image coordinates;
only storage addressing uses tile coordinates. Sample batches adapt to measured
dispatch latency. `map_async` readbacks overlap the next slot's computation;
a bounded result channel supplies one host collector.
Each slot retains its measured dispatch plan between jobs, avoiding a repeated
cold ramp through many small submissions for every tile.
No mutex protects a
full image during shading. GPU output memory depends on the slot dimensions,
not the export resolution. Scene resources remain resident and shared between
the GPU slots.

`--backend hybrid` additionally requires a distinct Vulkan software CPU adapter
(for example lavapipe or SwiftShader). It executes the same WGSL transport, including
materials, volumes and first-hit guides, with one independent slot and the
same queue. The CPU path is not a second, simplified Rust integrator. The OS
software driver schedules CPU execution; Rayon remains used by the existing
CPU reconstruction and simulation passes. Following the GPU-only direction,
`auto` never starts this worker, regardless of image size. CPU/hybrid modes
remain explicit diagnostics for existing checkpoints and measurements.
CPU pipeline preparation and a discarded JIT warmup run in a separate thread
before it claims any tile. GPU work starts immediately. If the image completes
or is cancelled while the native JIT is still preparing, that bounded background
preparation can finish without delaying the output; it cannot commit image data.
CPU execution/preparation is serialized per process to prevent competing software
driver compilations and exports from saturating the host simultaneously.
On Windows, the GPU submission worker requests `THREAD_PRIORITY_ABOVE_NORMAL`
so CPU shading threads cannot continually delay new GPU submissions. This
[thread scheduling priority](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setthreadpriority)
applies only to that worker for its lifetime; process and UI priorities are
unchanged. Other platforms retain their normal scheduling policy.
On Windows, `scripts/with-vulkan-cpu.ps1` adds a user-supplied CPU driver manifest
or an installed SwiftShader runtime to the child process's driver list. It
resolves DLL paths in a private temporary manifest, preserving the vendor file
and system configuration. WARP is excluded after native DX12 crashes during
validation on the target machine. No hybrid speedup is assumed: software driver
and scene performance vary.
The launcher refuses elevated execution because the Vulkan loader ignores
environment driver overrides in administrator processes. It preserves explicit
driver lists while appending the CPU manifest to their effective selection.
`syfrack probe --cpu` checks the software adapter with a small compute kernel,
without compiling a scene. Studio displays preparation, actual CPU/GPU tile
counts and an unavailable-CPU diagnostic.
CPU export variants freeze only the immutable dimension and exactly zero-valued
scene features. Pure WGSL helpers keep unreachable defaults such as disabled
parallax's 0/0 out of shader-creation constant evaluation; the native compiler
can inline them and eliminate absent transports. Two separate variant cache
buckets retain the existing per-topology LRU. Enabled feature values remain
uniforms, and global pixel/sample coordinates remain dynamic. CPU workgroups
follow the software adapter's SIMD subgroup size, capped at eight invocations;
pixels have no workgroup-shared state.
Explicit `gpu` retains the existing full-image engine. Global guiding and flame
histograms use that engine in `auto`; explicit tiled/hybrid requests reject them.

Only completed tiles are committed to the host image and checkpoint. A pause
or cancellation stops workers between dispatches, closes the bounded channel
before joining them, and discards uncommitted work. Resume recomputes those
tiles. Tiled checkpoints use versioned metadata in `state.ron` and a streamed
`export.bin` payload with exact byte-count and checksum validation. Directory
replacement keeps the previous checkpoint recoverable on failure. The common
export API reads both resident and tiled checkpoints; the lower-level
`RenderSession` deliberately rejects the latter's normalized pixel payload.
New tiled checkpoints also retain adapter/driver, workgroup and actual shader
implementation identities; a changed native arithmetic producer is refused.
An explicit resume backend override accepts native differences and retains the
previous producers alongside the new implementation. Already completed tiles
are preserved exactly. CPU-only is available for diagnostics and recovery;
hybrid rendering still shares one queue between both processors concurrently.
Each tiled export keeps bounded host-clock dispatch accounting. A short mutex
updates timestamps at submission and completion; it is released during driver
work and waiting. The CPU/GPU intersection measures overlapping in-flight
commands, including driver overhead, and does not claim hardware ALU occupancy.
Console phase markers distinguish warm-up from timed benchmark passes. The benchmark rejects
a concurrent-throughput claim if either worker contributes no tile or its
dispatch intervals never intersect the other worker's.
Legacy version-1 states remain readable without this provenance. Native hybrid
CPU/GPU output is not qualified as portable bit-exact. The strict diagnostic
gate, native quality metrics and cumulative-throughput benchmark are described
in [Determinism](DETERMINISM.md).

Independent instances have a CPU-built, balanced BVH with signed conservative
AABB distances. GPU traversal uses preorder escape indices, without a fixed
traversal stack. Leaves retain original placement indices for deterministic
ties; every detailed result is still clamped to its rotated/scaled fence.
`PreparedGraph` reuses plans and BVHs for CPU evaluation. Ordinary parameters
retain a 32 KiB uniform prefix; overflow lives in dimensioned, unfiltered RGBA32F
data textures, avoiding the uniform-buffer limit
without adding storage-buffer bindings. Limits are 16,384 copies per node,
65,536 placements per nested branch and 524,288 packed scalars per graph.

This bounds output-buffer residency; it does not page arbitrary geometry or
volume bricks from disk. Current fractal details are implicit, and existing
HDRI/volume preparation retains its own budgets. Rendering independent chunks
and discarding the rest of a scene would change visibility, shadows and indirect
light. A future out-of-core asset system needs explicit resource dependencies
and a residency protocol before such chunks can safely be evicted. Host image
planes currently have an 8 GiB admission limit; readback copies and denoising
require additional RAM.

## GPU compute redesign: target and acceptance gates

Implementation: `--backend wavefront` now runs seven compute entry points:
start, generate, prepare-indirect, intersect, shade, compact and commit. New 3D
`auto` exports select it; `tiled` and `gpu` retain the reference integrator.
The shade stage currently fuses medium processing, normals, direct lighting,
shadow queries and BSDF continuation. The original bounce operations are reused
with persisted RNG, MIS, medium and first-hit state. Intersections still march
the complete segment; marching quanta and separate visibility queues below are
optimisation targets, not implemented claims.

Paths/hits use SoA planes, queue entries are IDs, and compaction performs one
atomic range reservation per workgroup with uniform scans. An argument-building
pass writes the indirect dispatch buffer on the GPU. A bounded command chain
contains every bounce of a sample; no host wait/readback occurs between stages.
At the batch boundary a 16-byte status read verifies queue capacity and complete
termination before the result is accepted. Film copies preserve the existing
readback/guide/checkpoint planes and ordered sample accumulation. Arenas are
reused, capped at 512 MiB per slot and checked against device binding limits.
New wavefront blocks reach 256x256 on images with at least 65,536 pixels,
independently of the logical checkpoint cells. A separate transport identity
prevents accidental mixing on resume; legacy configurations default to the old
integrator. Initial GPU tests cover fog layers, guides, clipped tiles, checkpoint
replay, cutout null events and dielectric medium state. WGSL validation and SPIR-V
generation are both tested. The [Orfevrerie benchmark](benchmarks/orfevrerie-20261018/README.md)
measures a 1.481x warmed speedup with identical HDR values on the Quadro target;
it does not establish throughput on every modern GPU.

Automatic CPU rendering has been disabled. Orfevrerie's
failed four-sample hybrid test is retained; its cancelled 128-sample test yields
no result. No rendering parameters were reduced to establish a speedup.

The reference 3D entry point already uses compute shaders. `radiance` combines
intersection, normals, materials, direct lighting, fog and path continuation;
each invocation loops over its sample batch. `Renderer::dispatch` submits and
waits for each dispatch. Compact export blocks expose at most 128x128 pixel
invocations at once. These are concrete design constraints, not proof of a
particular register count or GPU occupancy. Those require measurements on the
target device. There is no portable wgpu operation that assigns a job to a
specific physical SM/CU.

The target is a **GPU-resident, selective wavefront integrator**. Tiles remain
units of film storage and checkpointing. Active ray batches become independent
units of GPU execution; several tiles can supply one batch. Keep inexpensive
coherent operations fused, and split expensive divergent operations only when
the bandwidth and dispatch costs are repaid. The existing analytic 2D path
does not acquire unnecessary 3D queues.

| Compute stage | Input and output | Contract |
| --- | --- | --- |
| Generate | Pixel/sample IDs to active path IDs | Preserve full-image camera coordinates and sampling identities |
| Intersect | Active rays to hits, misses and unresolved rays | Preserve distance bounds, ray endpoints and exhausted-ray semantics |
| Medium | Segments to volume contributions and scattering/continuation events | Preserve free-flight distribution, transmittance and MIS |
| Shade | Surface events to local contributions, shadow requests and continuation rays | Specialise enabled material families; retain PDF and medium state |
| Visibility | Shadow requests to visibility/transmittance | Opaque visibility uses distance-only evaluation; transparent coverage retains its semantics |
| Gather and compact | Fixed contribution slots and surviving path IDs | One writer per path/sample; deterministic contribution order; no floating-point atomic sum |
| Commit | Completed samples to film planes | Add samples in ascending order; keep depth, guides and volume layers consistent |

Queues contain path IDs; path data lives in reusable, aligned SoA arenas.
Separate frequently accessed origin/direction data from colder PDF, material,
medium, RNG and first-hit metadata. Avoid copying the whole path at each queue
transition. Queue counters and indirect-dispatch arguments stay on the GPU.
Use ping-pong queues, explicit bounded capacities and overflow detection.
Overflow must retry a smaller batch before committing it; it must never drop
a ray, a shadow request or a contribution. Light fan-out must be bounded by
staging requests across rounds, rather than allocating an unbounded list.

Producer, count/compact, argument-building and consumer stages use ordered
compute passes. Record several passes in one command buffer and submit bounded
chains. `dispatch_workgroups_indirect` consumes GPU-generated sizes; host code
still records the pass graph. No shader launches arbitrary new kernels. No
cross-workgroup spin barrier or assumption that every workgroup is resident
is permitted. `workgroupBarrier` cannot synchronise the whole GPU. This avoids
both scheduler deadlocks and a CPU readback between every bounce.

Use subgroup scans/ballots when the negotiated device supports them, with a
workgroup-scan implementation otherwise. Reserve one contiguous output range
per workgroup, then scatter surviving path IDs into it; avoid a global atomic
reservation for every surviving ray. Inactive edge lanes participate in scans
with a zero predicate and still reach all required barriers. Subgroup size is a device/compiler
property, never a hard-coded 32. Test 64, 128 and 256 invocations per workgroup
within device limits, choosing per kernel by measured throughput. More threads
or higher occupancy alone is not a success criterion. wgpu currently requests
no optional device features in either Studio or headless creation; timestamp
queries and subgroup capabilities must be negotiated in both before use.
Separate layouts for wavefront passes must respect storage-binding limits;
adding a binding for every conceptual queue to the existing layout is unsafe.

For fractal intersections, prototype fixed marching quanta (for example 16,
32 and 64 steps) followed by survivor compaction. Resume every part of the
current marching state: total step count, t, omega, previous radius, step length,
candidate t/error/sample and bound flags. Resetting those between quanta changes
geometry. Retain a full-loop variant when compaction traffic costs more than
the divergence it removes. Compile a distance/flags-only evaluator for traversal
and visibility; evaluate traps and shading attributes at the accepted hit when
equivalence is established. Do not infer a safe ray start from a low-resolution
cone prepass without a conservative geometric proof.

One path owns one pixel/sample until completion. Store its exact RNG state and
sampling position across transitions. During the first port, preserve the
existing draw sequence and light contribution order. Deferred shadow results
write ordinal slots, then one gather kernel reduces them in the reference
order. Keep at most one sample per pixel active initially; independent pixels
provide parallelism without unordered sample sums. A counter-based sampler or
multiple simultaneous samples would be a separately versioned change.

Working capacity derives from an explicit byte budget and the measured aligned
strides of paths, hits, shadow requests, queues and film planes. Check each
buffer against max_buffer_size/max_storage_buffer_binding_size and check index
arithmetic before allocating. Increase active batch size independently of tile
dimensions until throughput plateaus, memory limits intervene or dispatches
exceed the latency budget. Do not allocate a path for every pixel of an 8K image.
Keep readback/checkpoint staging pooled; map only small progress counters during
normal tracing. Global denoising, exposure and colour development still occur
after HDR assembly.

Implementation order and gates:

1. Add optional GPU timestamps and a reproducible GPU-only benchmark. Separate
   preparation, GPU pass time, host waiting, queue/compaction traffic, readback
   and development. Record adapter, driver, workgroup and peak buffer bytes.
   Hardware tools are required for register spilling, occupancy and stalls;
   host dispatch duration cannot establish those metrics.
2. Measure immutable feature specialisation and distance-only evaluation against
   the current shader. Keep the same estimator, transport and sample budget.
3. Port path continuations, visibility and gather into the bounded pass graph;
   then evaluate marching quanta and cost/material binning. Keep a monolithic
   variant for workloads where the measured wavefront overhead is greater.
4. Integrate scheduling, UI cancellation and checkpoints only after raw HDR and
   every auxiliary plane pass the reference tests. New in-flight state requires
   a versioned checkpoint contract; do not reinterpret old normalized tiles as
   new sample sums. Old checkpoints retain their supported legacy path.

Use Orfevrerie with fog and metal, a cheap sky-heavy scene, silhouette-heavy
fractals, large instance BVHs, HDRI, transparent/volumetric materials and clipped
image edges. Measure cold preparation separately and repeat interleaved warmed
baselines. Require finite output, complete coverage, bounded queues, cancellation
and resume consistency. Apply the declared HDR quality gate, report worst local
errors and preserve all lighting/MIS contributions. Qualify on both a modern
GPU and the Quadro compatibility path; results from the Quadro alone cannot
certify modern-GPU throughput. No speedup factor is promised before these gates.

The wavefront decision is supported by [Laine, Karras and Aila's GPU study](https://research.nvidia.com/publication/2013-07_megakernels-considered-harmful-wavefront-path-tracing-gpus)
and [PBRT's discussion of kernel fusion and bandwidth costs](https://pbr-book.org/4ed/Wavefront_Rendering_on_GPUs/Mapping_Path_Tracing_to_the_GPU).
[AMD's occupancy guidance](https://gpuopen.com/learn/occupancy-explained/)
explains why workgroup size and registers require measurement. Pass ordering,
barriers and subgroup use must follow the [WGSL specification](https://www.w3.org/TR/WGSL/)
and [wgpu indirect-dispatch API](https://wgpu.rs/doc/wgpu/struct.ComputePass.html#method.dispatch_workgroups_indirect).

## Colour and export

The renderer retains linear HDR values. `syfrack-color` applies exposure,
tone mapping and the chosen output transform for PNG. Raw EXR remains available.
Native denoising uses geometry guides; an optional CPU Open Image Denoise child
process can filter supported opaque exports without replacing the raw EXR.
Tile workers never apply a Look. Denoising/reconstruction, global auto-exposure,
tone mapping, vignette and grain run on the assembled full image. Raw EXR stays
scene-linear.
EXR blocks compress in parallel and are written in increasing order, making the
encoding reproducible for identical pixels instead of following thread completion.

## Studio

`syfrack-app` combines winit, egui and wgpu. `AppState` owns scene edits and undo
history; UI preferences stay outside scene files. Tool windows share the egui
context and GPU. Their IDs remain stable when their translated titles change.

Background final renders use a scene snapshot. Editing another composition or
changing the UI language does not change an active render's samples.
Overall progress and ETA use completed pixel-samples, including clipped edge
areas. The minimum sample count remains a convergence diagnostic; it cannot
represent progress when workers complete whole tiles. GPU dispatch/readback
waits target their own submission indices, so later UI submissions do not extend
those waits. The readback slots share measured dispatch plans. A converged,
unchanged preview no longer repeats its auto-exposure readback every second.
