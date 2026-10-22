# Native ocean surface

`crates/syfrack-render/src/shaders/ocean.wgsl` implements a displaced, intersectable
interface and an analytic seabed. It is part of the native path tracer, rather
than an image filter or a screen-space reflection effect.

## Geometry and foam

Twelve directional gravity-wave bands use irregular wavelength ratios, amplitudes
and phases. Slow transverse envelopes form wave packets, so the surface does not
repeat a single grid of sine waves. Their vertical second harmonic sharpens
crests. The time parameter drives phase
with the gravity-wave dispersion relation and is saved with the scene. It stays
fixed during accumulation. The interface has no horizontal folding or overhangs.

A conservative height slab, global derivative bound and a bounded Taylor
remainder constrain ray tracing, including the derivatives of the packet
envelopes. Wave directions, amplitudes, time phases and
derivative bounds are precomputed once per uniform update, not per ray step.
Wave derivatives give geometric normals; capillary-scale shading normals are
attenuated by the transported pixel footprint. First-hit depth and normals include
the ocean, so export denoising respects shore edges.

Crest foam follows the wave phase with spatially varying patches. Shore foam
follows the scene's signed distance estimate with irregular wash, instead of
concentric sine stripes; a nearby bounding proxy is resolved before it can create
foam. Unwrapped world-space noise supplies capillary normals, roughness variation
and fragmented foam. Foam detail blends towards its mean at a distance. The shore model is a distance-based
approximation, not a simulation of breaking waves against solids.

## Light transport

The surface combines rough GGX dielectric reflection, diffuse foam and smooth
Snell transmission. Exact unpolarised Fresnel handles total internal reflection.
Transmission probabilities are included in reflection PDFs and MIS weights;
radiance transport includes the squared relative refractive index. Reflection
paths can hit real geometry, area emitters, the procedural sky and HDR environments.

Underwater paths apply Beer–Lambert extinction from RGB absorption plus scattering.
A single-scattering source approximation adds the selected water colour. Sun and
area shadow paths use straight interface crossings with Fresnel loss and extinction.
This approximation permits illumination of submerged geometry without opaque water
shadows, but does not bend NEE rays or generate focused caustics. Diffuse foam is
not accounted for in the straight shadow approximation. Transmission roughness is
represented by the resolved wave normals; the GGX microfacet roughness affects
reflection. The seabed is flat and configurable, while nearer graph geometry can
replace it locally.

## Storage and controls

`Seascape.surface: Option<OceanSurface>` defaults to `None` for compatibility.
Partial surface settings use `OceanSurface::default()`. Values are validated
through the scene parameter system. Wind direction is degrees in scene files,
converted to radians in the packed uniform. The GUI uses the same validated
parameters and undo history. Three or more bounces are recommended.

## Reproducible checks

```
cargo test -p syfrack-core seascape --offline
cargo test -p syfrack-render --lib --offline
cargo test -p syfrack-render --test gpu_ocean --offline -- --ignored --test-threads=1 --nocapture
cargo run -p syfrack-render --example ocean_lab --offline -- renders/studies/ocean 32 800
```

The GPU regressions verify actual wave depth/normal changes, reproducible time
variation, channel-dependent absorption of a submerged emitter, transmitted
sunlight on the seabed, foam confined to the shore, finite non-negative output,
and total internal reflection from below. On the Quadro K3100M/Vulkan, the flat
clear-water test returned linear radiance 6.588 from an underwater source of 12.
With absorption `(0.5, 0.1, 0.02)` /m, RGB transmission ratios were approximately
`(0.323, 0.797, 0.956)`. The wide-source total-internal-reflection test measured
11.925. The updated foam
shader measured 0.220 at the shore against 0.0119 offshore.

The grazing regression also exercises 4 m waves with a 1 m wavelength. Its
initial 128-step limit left holes; the final steepness-dependent search budget
passes both that extreme and the regular swell with zero misses. All four ocean
GPU tests and ten architecture/material regressions pass on the final ocean and
rock shader. These include seeded eroded rocks matching the CPU distance
estimator across negative coordinates, grazing intersections and transport
through the water. Final logs are stored in
`docs/benchmarks/coast-2026-10-10/`; earlier ocean/lighting logs remain in
`docs/benchmarks/ocean-2026-10-10/`.

`renders/ocean-surface.png` is a native 640 × 360, 32-sample technical water
study, with linear EXR and the rendered RON beside it. Its deliberately simple
solids expose shoreline foam and submerged markers; it is not the commercial
city composition. The editable scene is `scenes/environments/ocean-surface.ron`.
The current Elysium project uses the new surface, three families of eroded rock
foundations and 120 irregular shore boulders. The coastal rock material darkens
and smooths wet surfaces near the waterline. Its complete native control preview
is `renders/studies/coast-2026-10-10/elysium-portable-preview.png`, at 320 × 180,
8 samples and 2 bounces. It is not the publication render. The portable coast
package includes the full scenes, current GUI/CLI, runtime DLLs and console-free
HD launchers for 1920 × 1080 / 256 samples / 4 bounces. HD buffers were exercised
locally, and pause/resume reproduced an uninterrupted render's raw EXR exactly.
The target GTX 1050 was not available for a hardware test.

## References

- [GPU Gems, Effective Water Simulation from Physical Models](https://developer.nvidia.com/gpugems/gpugems/part-i-natural-effects/chapter-1-effective-water-simulation-physical-models): directional wave superposition and analytic derivatives.
- [PBRT 4, Dielectric BSDF](https://pbr-book.org/4ed/Reflection_Models/Dielectric_BSDF) and [Specular Reflection and Transmission](https://www.pbr-book.org/4ed/Reflection_Models/Specular_Reflection_and_Transmission): Fresnel, Snell refraction and radiance scaling.
