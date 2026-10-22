# Showcase projects

These are editable scenes, not finished render/checkpoint bundles. They use the
same native engine as Studio and the CLI.

## Breath of the Titans — 2D

`breath-of-the-titans.ron` explores a Julia field with flowing blue filaments,
golden recursive boundaries and a rotated frame. The documentation preview
comes from a native 3840 × 2160 render with 256 samples per pixel.

```text
syfrack render scenes/showcase/breath-of-the-titans.ron --out renders/breath-study --width 1280 --height 720 --samples 32
```

## Atlas — Guardian of Infinity — 3D

`atlas-guardian-of-infinity.ron` combines a stretched recursive body, fine
orbital structures, metallic materials and a small figure for scale. It is a
demanding geometry and lighting study.

```text
syfrack render scenes/showcase/atlas-guardian-of-infinity.ron --out renders/atlas-study --width 640 --height 800 --samples 8 --bounces 2 --steps 512 --pixel-tolerance 0.5 --denoise
```

The large study used a 1920 × 2400 portrait, four bounces, 1024 tracing steps and
a 0.125-pixel primary geometry tolerance. Its render was intentionally paused
at 100–101 samples per pixel out of a target of 128. A historical Studio illustration
used that saved native image as a labelled reference; this source tree does
not include its checkpoint or claim a completed 128-sample render.
