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

## Colour and export

The renderer retains linear HDR values. `syfrack-color` applies exposure,
tone mapping and the chosen output transform for PNG. Raw EXR remains available.
Native denoising uses geometry guides; an optional CPU Open Image Denoise child
process can filter supported opaque exports without replacing the raw EXR.

## Studio

`syfrack-app` combines winit, egui and wgpu. `AppState` owns scene edits and undo
history; UI preferences stay outside scene files. Tool windows share the egui
context and GPU. Their IDs remain stable when their translated titles change.

Background final renders use a scene snapshot. Editing another composition or
changing the UI language does not change an active render's samples.
