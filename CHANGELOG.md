# Changelog

## 0.2.0 — 22 October 2026

- A single typed creation graph and document for 2D/3D projection, patterns,
  GPU flame fields, simulations, palettes and geometry, with shared undo/redo.
- Persistent Simple/Advanced interface selection. Focused artistic controls
  edit the same graph; Advanced remains the first-launch default.
- Revised node workflow: path library, typed connections in both directions,
  insertion/reconnection, complete recipes, source precision and compact framing.
- GPU control preview for 3D graph editing without per-edit shader compilation;
  specialized material/light preview remains available explicitly.
- Mosaic photo reconstruction with crop/rotation, adaptive refinement, priority
  painting, comparison, portable projects and independent edit history.
- GPU-only automatic export; explicit CPU/hybrid diagnostics and checkpoint
  provenance remain available with their documented arithmetic limits.
- Render/export window reorganized around format, quality, destination and
  exact output filenames, with visible launch/pause actions and clearer progress.
- Fixed stale completed-render previews: the final image, including the last
  tiles and optional denoising, is sent before the Finished event.
- Source preparation includes editable study scenes, native HDR assets, regression
  inputs, linked guides, figures and
  benchmark evidence. Synchronization preserves independently edited files;
  inventories, packaging documentation dependencies and regression checks are updated.
- Hosted checks now include CLI and Mosaic project round trips. Legacy GPU test
  scene constructors initialize the new creation fields, keeping all targets buildable.

- CLI `--version` / `-V` reports the Cargo package version, including without a subcommand.

This source release is prepared for the owner to publish as `v0.2.0`.

## 0.1.0 — Initial public source preparation

- Initially separate 2D and 3D workshops with progressive previews and independent histories.
- Complex-plane fractals, fractal flames and node-based recursive 3D forms.
- Procedural materials, physical emitters, HDRI environments and experimental volumes.
- 16-bit PNG, linear EXR and RON scene exports with checkpoint pause/resume.
- English by default, a persistent English/French choice and a compiled message catalog.
- English Forme source export with legacy French syntax compatibility.
- Locale-aware numeric controls and WASD/ZQSD free-camera navigation.
- Source packaging, build instructions, localization audit and hosted CPU/UI checks.

This is an experimental source release. Binary distribution and additional
platform validation are separate steps.
