# Contributing to SyFrack

Bug reports, rendering comparisons, scene examples and focused patches are
welcome. This is an experimental renderer: clear reproduction steps and image
evidence help more than general claims about quality or performance.

For a bug, include the OS, GPU, driver/backend, SyFrack revision, relevant render
settings and a minimal scene. Remove personal paths and external private assets
before sharing. Describe expected and actual behaviour and attach a small crop
if the problem concerns detail, noise or colour transitions.

Before submitting code, run the checks in [BUILDING.md](docs/BUILDING.md).
Add a regression test when changing numerical behaviour or state semantics.
For GPU changes, compare representative scenes on actual hardware and keep
geometry, exposure, sample count and seed controlled. Report render time and
image differences together.

Preserve these contracts:

- User scenes and names must not change when the UI language changes.
- Graph parameter paths and serialized enum names are stable identifiers.
- The raw EXR remains available when an export filter changes the PNG.
- A scheduling optimisation must not silently change samples or shading.
- New UI text belongs in the shared English/French catalog.
- Keep generated renders, build caches, downloaded runtimes and credentials
  outside commits.

See [Architecture](docs/ARCHITECTURE.md) and
[Localization](docs/LOCALIZATION.md) for the corresponding code paths.
Contributions to the project are under its MIT license.
