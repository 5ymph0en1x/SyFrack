# Orfevrerie: CPU discovery and export scheduling, 2026-10-18

The user reported a 1920 x 1080, 128-sample Studio export showing 0% after
36 minutes, with `no fallback adapters` for the CPU worker. The provided scene
is `scenes/orfevrerie.ron`; its file was not edited.

## Confirmed faults and fixes

The running Studio process had `TokenElevation = true`, while the diagnostic
shell was not elevated. The [Vulkan loader's documented security policy](https://github.com/KhronosGroup/Vulkan-Loader/blob/main/docs/LoaderDriverInterface.md)
ignores `VK_ADD_DRIVER_FILES`, `VK_DRIVER_FILES` and `VK_ICD_FILENAMES` for
elevated applications. Printing the CPU DLL path in the launcher did not prove
that the driver was accepted. The launcher now refuses that configuration
before opening Studio and explains how to launch from ordinary PowerShell.
Explicit inherited driver lists are retained with the CPU manifest appended.
A real `probe --cpu` through the normal launcher found SwiftShader Device
(Subzero), Vulkan, and passed the 256-value compute check (last value 511).

Progress incorrectly used the least-advanced tile. A whole-tile renderer can
have many completed tiles while that minimum remains zero. The progress bar,
average sample count and ETA now use completed pixel-samples, weighted by the
actual area of clipped edge tiles. Studio also displays CPU preparation,
completed CPU/GPU tile counts, or CPU unavailability with its diagnostic.

Dispatch and staging waits now target their own submission indices rather than
the newest submission on Studio's shared GPU queue. Later UI submissions no
longer extend those waits. An unchanged, converged preview no longer rereads its
image for auto-exposure every second. CPU warm-up caching distinguishes colour
rendering from material-guide pipelines, so enabling denoising later cannot
mistake those uncompiled pipelines for warmed ones.

GPU batches grow from 64 x 64 to a bounded 128 x 128 on images with at least
65,536 pixels. CPU/checkpoint jobs remain at most 16 x 16. Two GPU slots share
the measured dispatch plan instead of independently repeating the cold ramp.
The latency backoff, per-pixel sample ordering and full-image camera/RNG
coordinates remain in use. Larger in-flight blocks can require more unfinished
work to be recomputed after a pause; committed tiles remain preserved.

## Measurements and limits

GPU: Quadro K3100M, Vulkan, NVIDIA 426.78. The performance experiment uses the
provided file at 384 x 216, 8 samples/pixel, 4 bounces, 512 march steps, pixel
tolerance 1, denoising enabled and a requested tile size of 256. This reduced
image has the original 16:9 aspect. It does not establish full-HD throughput or
reproduce unspecified edits made in Studio.

| Measurement | Render seconds | Dispatches |
| --- | --- | --- |
| Previous tiled engine, three fresh processes | 21.8, 21.8, 21.9 | 206 each |
| Updated engine, three fresh processes | 21.445, 22.124, 21.577 | 195 each |
| Updated engine, two additional runs in the first process | 19.193, 19.215 | 195 each |

The fresh-process medians differ by about 1%, within the variability of this
small experiment. This does **not** establish a substantial isolated-GPU
speedup. Comparing warmed runs to fresh processes would overstate the change.
The direct improvement demonstrated here is fewer dispatches with identical
HDR values. CPU availability and the Studio-specific UI/readback fixes are
separate from these GPU-only headless measurements.

All five updated runs compare against the saved old GPU image: **zero differing
RGBA components, RGB/alpha RMSE zero**. They use the same scene, sample count,
transport and geometry tolerance; no lighting or quality setting was removed.
This verifies the measured GPU scheduling change. It is not a certificate of
native CPU/GPU bit equality or a portable bound on all scenes.

Timings labelled `render_seconds` are the engine's render interval, excluding
initial session construction, global denoising and encoding. The first updated
process takes 54.727 seconds including session preparation, render and readback;
the report preserves both values. GPU output/staging buffers with fog and
guides occupy 12,845,056 bytes, excluding shared scene resources and host planes.
No compiler or second render ran during these measurements. Run order was
updated/cached runs, old fresh-process runs, then two more updated fresh runs;
OS noise and thermal effects are not controlled statistically.

Raw data: [three updated runs](candidate-three-runs.ron),
[fresh process 2](candidate-fresh-2.ron), [fresh process 3](candidate-fresh-3.ron),
[old baseline console log](baseline.log).

Baseline executable SHA-256:
`D406E308A01B834FAFEB074D361DE563E56A4F73B3B215880EC44F1E20C46D7B`.
Profiling executable SHA-256:
`0D91584683389B56A566A8145746435A8569F5A104AD2F17FBC550E53BFC1D6A`.
The later CPU guide-warm-up correction does not change the GPU path measured by
this executable.

From the workspace root, after building `export_profile`:

```powershell
.\target\debug\examples\export_profile.exe scenes/orfevrerie.ron target/orfevrerie-before-tiled.exr target/orfevrerie-profile.ron 384 216 8 3
```

The reference EXR is a local test output, not a checked-in binary asset. The
profiling command refuses changed HDR values and preserves its report on failure.

## Native hybrid quality at low sample counts

The actual scene was also measured at 192 x 108, 4 samples/pixel, 4 bounces,
512 march steps, without denoising, after warming both devices in the same
process. A CPU-only diagnostic frame was measured separately. No compiler or
second render ran during the measurements.

| Backend | Seconds | Completed CPU / GPU tiles |
| --- | --- | --- |
| GPU | 2.359 | 0 / 84 |
| CPU | 21.251 | 84 / 0 |
| Hybrid | 2.136 | 8 / 76 |

The hybrid speedup is 1.104x, with 2,059 ms of overlapping host-observed
dispatch intervals. This establishes concurrent contributions, not hardware
ALU utilisation. The assembled hybrid image has relative linear-HDR RMSE
4.565%, maximum absolute channel error 0.972 and unchanged alpha. **It fails
the default quality gate** (RMSE <= 0.0001 + 0.01 * reference RMS).
The all-CPU diagnostic has relative RMSE 147.6%, dominated by sparse large
differences (maximum absolute channel error 56.581). These values are not a
valid quality certificate, even though the hybrid timing is faster. The report
is retained as [failed 4-sample qualification](hybrid-4spp-failed.ron).

The production sample count must be checked independently; low-sample path
tracing differences cannot be extrapolated to 128 samples, another camera or
full-HD output. The `hybrid_quality` example measures the GPU reference and
the assembled hybrid image without requiring an expensive all-CPU full frame.
It writes its report before enforcing the unchanged quality gate.

The 128-sample qualification was stopped when the user abandoned CPU rendering.
It produced no report and establishes no quality or performance result. Ordinary
`auto` exports now use the GPU alone, without discovering or warming a CPU ICD.

## Implemented GPU wavefront integrator

New 3D `auto` exports now select the GPU wavefront engine. Explicit `tiled`
retains the previous shader as a reference. The new engine persists paths and
hits in SoA buffers and uses GPU-generated indirect dispatches through separate
intersection, shading, compaction and commit stages. Medium and shadow work is
currently fused into shading. Queues, film and staging remain pooled and bounded.

The GPU-only benchmark used the supplied scene at 384 x 216, 8 samples/pixel,
4 bounces, 512 march steps, tolerance 1, without denoising. Both engines were
warmed before timing; measurements include session creation, rendering and
readback, but exclude encoding. Runs alternate order in the same process. No
compiler, CPU renderer or second render ran during the timed comparisons.

| Run | Previous tiled GPU (seconds) | GPU wavefront (seconds) |
| --- | --- | --- |
| 1 | 19.3067 | 13.0159 |
| 2 (wavefront first) | 19.2178 | 12.9685 |
| 3 | 19.0912 | 12.9724 |
| Median | 19.2178 | 12.9724 |

The median speedup is **1.4814x**, or **32.5% less time**. All three raw linear
HDR comparisons have **zero differing RGB components, zero RGB/alpha RMSE and
zero maximum error**. Sampling, light transport, geometry tolerance and the
scene file are unchanged. This qualifies this measured setup, not all scenes,
drivers, full-HD throughput or portable bit identity.

Output/staging/working buffers increase from 8,650,752 bytes to 85,983,424 bytes
(about 82 MiB), excluding shared scene resources and host image planes. Larger
256x256 GPU batches are part of the new engine measured here; the comparison
does not isolate kernel decomposition from batching. Latency backoff remains
enabled. The GPU driver schedules SM/CU occupancy; these wall times do not
measure occupancy or register pressure.

Raw report: [three interleaved GPU comparisons](wavefront-three-runs.ron).
Benchmark executable SHA-256:
`7CB92D40FDDF0DB14C2909D95D0418995FBD8F361E3532D66CB82319F98924B6`.

```powershell
cargo build --locked -p syfrack-render --example wavefront_benchmark
.\target\debug\examples\wavefront_benchmark.exe scenes/orfevrerie.ron target/orfevrerie-wavefront.ron 384 216 8 3
```

The benchmark writes its report before enforcing the HDR quality gate and
refuses any CPU contribution. Regression tests also cover clipped tiles,
volume components, first-hit guides, exact checkpoint replay, cutout null
events and dielectric medium state. All eleven bundled graphs validate through
WGSL and SPIR-V generation for the new entry points.

Final regression checks:

* `cargo test --locked -p syfrack-render --lib`: 114 passed, one optional OIDN
  runtime test ignored. This includes all eleven graphs and all seven SPIR-V
  entry points, comment-safe transport delimiters and 256x256 queue coverage.
* `cargo test --locked -p syfrack-render --test gpu_export -- --ignored --nocapture --test-threads=1 --skip hybrid --skip explicit_backend_resume`:
  seven GPU-only tests passed; software CPU/hybrid rendering was excluded after
  the user's change of direction. The wavefront checkpoint test compares the
  resumed output against a fresh `auto` export, verifying default selection.
* Locked workspace/all-target checking, formatting and tracked diff whitespace
  checks passed.

Studio and CLI were rebuilt successfully. A final CLI smoke export of Orfevrerie
with `--backend wavefront --denoise`, 96x54, 2 samples, 4 bounces and 512 steps
completed and wrote PNG, EXR and scene metadata. Reported tracing time was 2.3 s;
this small cold-process smoke test is not part of the performance comparison.

Final executable SHA-256 values:

* `target/debug/syfrack.exe`:
  `52F4F14EED3D7AE43F9156C294161F8D041724D423B1B94ED03C62C4BA9AAC14`
* `target/debug/syfrack-studio.exe`:
  `BC5A68FACBA96D7883EE998CCE13CB4CBD285AB856BBBCB5E76A8C18D67BB83E`
