# CPU + GPU export measurements, 2026-10-18

This local study uses a Quadro K3100M (Vulkan, NVIDIA 426.78) and the
SwiftShader Device (Subzero) Vulkan CPU adapter provided by Chrome
156.0.8078.36. Windows reports eight logical processors and
`Intel64 Family 6 Model 60 Stepping 3`. The CPU worker executes the common WGSL
integrator through the software driver. It is not the proposed native Rust / AVX2
packet tracer.

All reports include the complete scene, render configuration, actual arithmetic
producers, tile counts, wall times and linear HDR differences. Both pipelines
are warmed before timing. Timed exports include construction, tile pools,
scheduling, render and tile readback; they exclude cold JIT, final development
and image encoding. Ordinary first-use exports can finish on the GPU while the
CPU is still compiling. These warm measurements do not establish a cold-start
speedup.

## Warm results

The scene is [mandelbox.ron](mandelbox.ron). The comparable 384 x 256 runs use
8 samples/pixel, one bounce, 128 march steps, requested tile size 32, logical
CPU/checkpoint tiles 16, and GPU batches up to 64 x 64. Each report contains
three interleaved repetitions, alternating baseline order.

| Scheduler | GPU-only median | Hybrid median | GPU time / hybrid time |
| --- | ---: | ---: | ---: |
| Batched queue, normal GPU submission thread priority | 3.638 s | 3.994 s | 0.911x |
| Same queue, above-normal GPU submission thread priority on Windows | 3.564 s | 3.443 s | 1.035x |

See [before thread priority adjustment](batched-384.ron) and
[after adjustment](priority-384.ron). The adjustment applies only to the short
GPU worker thread; it lets that thread remain runnable when the software
driver's CPU pool occupies the machine. It changes no process-wide priority.

In the latter report the CPU completes 25-27 of 384 logical tiles while the GPU
completes the other 357-359. CPU-only median time is 53.768 s. The isolated-rate
sum estimates a 1.066x speedup for this workload, before contention and uneven
tile costs. The observed 1.035x ratio is a modest local improvement on three
repetitions, not a guarantee for other scenes or processors. Tile counts alone
do not demonstrate temporal overlap; version-3 reports add explicit host-clock
dispatch-overlap accounting.

The worst mixed-image HDR RMSE is 0.0017794464, or 0.4091% of reference RMS
0.43491881. It satisfies the declared criterion
`RMSE <= 0.0001 + 0.01 * reference_RMS`; alpha RMSE is zero. The largest local
RGB error is 0.555195. A global RMSE does not bound every pixel. The full CPU-only
diagnostic image has RMSE 0.012168276 (2.798%) and exceeds this same criterion;
it is recorded separately and must not be described as qualified to 1%.

The earlier one-repetition 192 x 128 reports [baseline](baseline-192.ron) and
[tail retirement](tail-192.ron) document the initial bottleneck. They have a
different job layout and resolution and are not comparable speedup claims
against the 384 x 256 runs. The temporary CPU-thread-count experiment is omitted
because compilation overlapped its measurements.

## Dispatch-overlap confirmation

The [instrumented version-3 report](overlap-384.ron) repeats the same workload
once after the accounting was added. This is a concurrency diagnostic, not a
replacement for the three-repetition performance study:

| Host-clock interval, relative to worker-pool start | First dispatch | Last completion | Dispatch wall time |
| --- | ---: | ---: | ---: |
| CPU, 30 dispatches / 27 logical tiles | 1.366 ms | 3114.869 ms | 3108.422 ms |
| GPU, 71 dispatches / 357 logical tiles | 0.794 ms | 3442.877 ms | 3419.021 ms |

The intersection of the individual CPU and GPU dispatch intervals is
**3086.868 ms**, within a **3447.700 ms** hybrid export. It is calculated from
active intervals, not merely the span between the first and last dispatch.
Isolated CPU/GPU baselines both record zero overlap. This confirms concurrent
in-flight work at the host/API boundary. The intervals also include driver
submission and waiting; they do not directly measure simultaneous hardware ALU
occupancy.

This run records GPU-only 3.593 s, CPU-only 51.484 s, hybrid 3.448 s, and the same
0.4091% mixed-image relative HDR RMSE. Both the concurrent-work gate and hybrid
quality gate pass. The full CPU diagnostic remains outside the declared 1%
criterion. The console explicitly labels warm-up, each isolated baseline and
the hybrid pass before each starts, avoiding ambiguity about which phase is
running while one processor is idle.

This diagnostic used debug-profile `syfrack.exe`, SHA-256
`D406E308A01B834FAFEB074D361DE563E56A4F73B3B215880EC44F1E20C46D7B`.
Validation of the instrumented sources: 109 renderer unit tests passed (one
optional OIDN test ignored), 49 colour unit tests passed, all eight GPU export
integration tests passed with the CPU driver, and the locked workspace check
for all targets passed. The checkpoint test verifies complete EXR/PNG byte
equality after a same-backend pause/resume.

## Reproduce

From the workspace root, after building the CLI:

```powershell
.\scripts\with-vulkan-cpu.ps1 -Executable .\target\debug\syfrack.exe -RenderArgs @('benchmark-hybrid', 'docs/benchmarks/hybrid-20261018/mandelbox.ron', '--out', 'target/hybrid-study.ron', '--width', '384', '--height', '256', '--samples', '8', '--bounces', '1', '--steps', '128', '--tile', '32', '--repeats', '3')
```

Run without simultaneous builds or other render jobs. The console distinguishes
CPU-only JIT warm-up, the isolated baselines and the simultaneous hybrid pass.
Version 3 additionally records CPU/GPU submission-to-completion intervals and
their intersection. These host-clock intervals include driver work and waits;
they are not GPU hardware timestamp queries or ALU utilization measurements.

The archived version-2 priority report was produced by debug-profile
`syfrack.exe`, SHA-256
`17E4F76DC938FF73C68BC53E620B8AA539B1C0218417D578731868E7C70E7CCA`,
with Rust `1.99.0 (b940084d7 2026-09-28)`. Workspace HEAD was
`ec7c93108498cec149f5b056f3f4c33842489b17` with uncommitted changes; HEAD alone
does not identify the tested implementation.
