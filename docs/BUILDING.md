# Building and testing

## Windows x64

Install Rust through rustup and the MSVC C++ build tools. Work from a short,
writable path. `rust-toolchain.toml` pins the toolchain; `Cargo.lock` pins Rust
dependencies. The first build downloads those dependencies. Offline builds work
only after the toolchain and crates have been cached.

```powershell
cargo build --release --locked --workspace --bins
```

The binaries are `target/release/syfrack-studio.exe` and
`target/release/syfrack.exe`. `scripts/build.ps1` wraps this command without
forcing CPU-specific instructions. It does not install a GPU driver or external
denoiser.

Launch from the repository root, or place `graphs/`, `scenes/` and `assets/`
beside a portable executable. Run `syfrack --version` to confirm the installed
version and `syfrack probe` to inspect the selected GPU.
Studio uses Vulkan or Direct3D 12. The CLI tries Vulkan, Direct3D 12, then OpenGL. Windows is the tested
distribution target; other OS/backend combinations need separate validation.

## Verification

```text
python scripts/i18n/audit.py
python -m unittest discover -s scripts/i18n -p test_*.py
python -m unittest discover -s scripts/package -p test_*.py
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo test --locked --workspace --lib
cargo test --locked -p syfrack-core -p syfrack-cli --tests
```

The library suite includes graph/scene, colour, checkpoint and headless UI
tests. GPU integration tests require a working local graphics backend and are
not run by the basic hosted CI job. Read a test file before enabling ignored
tests; some ignored developer helpers deliberately regenerate assets.

The historical GPU image regression uses the three committed EXR inputs in
`crates/syfrack-render/tests/fixtures/parity/`. Keep those files when copying
the source tree. The optional `guiding_profile` example also ships its three
small scene/reference pairs under
`docs/benchmarks/guiding-cost-2026-10-09/fixtures/`, including Menger's relative
`studio.exr` environment. Regenerate that synthetic environment with
`cargo run --locked -p syfrack-render --example hdri_lab -- docs/benchmarks/guiding-cost-2026-10-09/fixtures/menger --panorama-only`;
this mode writes only the CPU-generated panorama and requires no GPU.
These explicit inputs are
the exceptions to the generated-EXR exclusion.

For focused GPU checks, from a GPU-equipped checkout:

```text
cargo test --locked -p syfrack-render --test gpu_de -- --ignored
cargo test --locked -p syfrack-render --test gpu_canvas -- --ignored
cargo test --locked -p syfrack-render --test gpu_export -- --ignored --skip hybrid_ --skip explicit_backend_resume --test-threads=1
cargo test --locked -p syfrack-app --test gpu_final_render job_completes_and_matches_a_direct_render_bit_for_bit -- --ignored --nocapture
```

Hybrid export, throughput and cross-backend resume checks additionally require a Vulkan CPU adapter. On
Windows, PowerShell 7 can add an installed SwiftShader runtime for the test
process (or supply `-Icd` with a CPU Vulkan driver's manifest):

```powershell
.\scripts\with-vulkan-cpu.ps1 -Executable (Get-Command cargo).Source -RenderArgs @('test', '--locked', '-p', 'syfrack-render', '--test', 'gpu_export', '--', '--ignored', '--test-threads=1')
```

`gpu_parity::parity_lockstep_test` is a separate strict qualification gate. It
currently fails on native arithmetic and must not be counted as an ordinary
tolerance-based GPU smoke test. See [Determinism](DETERMINISM.md) for the report
command, the measured limits and the dependent phases it blocks.

The `Source checks` GitHub workflow checks the source and CPU/UI suite on Windows. Its checkout
step uses the official [actions/checkout](https://github.com/actions/checkout).
The separate `Windows release` workflow builds and publishes a Windows x64 ZIP
when a version tag is pushed; see [Publishing](PUBLISHING.md).

## Portable release archive

The release workflow uses the pinned Rust toolchain, `--release --locked`, the
explicit `x86_64-pc-windows-msvc` target and `-C target-feature=+crt-static`.
It does not use `target-cpu=native` or laptop-specific flags. The C runtime is
linked statically, so a separate Visual C++ redistributable is not bundled.

To reproduce the packaging locally from a clean source export/checkout:

```powershell
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --release --locked --target x86_64-pc-windows-msvc --workspace --bins
$revision = git rev-parse HEAD
python scripts/package/windows-release.py build --tag v0.2.0 --revision $revision --target-dir target --output-dir dist
python scripts/package/windows-release.py verify --archive dist/SyFrack-v0.2.0-windows-x64.zip --smoke-test
```

Replace the version with the tag matching `workspace.package.version`. Without
a Git checkout, supply the full 40-character source commit SHA explicitly.
The script refuses to replace an existing ZIP. The ZIP contains both binaries,
curated scenes, graph thumbnails, HDRI assets, guides, dependency/font license
texts, a dependency inventory, `BUILD_INFO.json` and `PACKAGE_MANIFEST.json`.
The external `SHA256SUMS.txt` checks the complete archive.

Archive verification checks every file and PE architecture/subsystem, rejects
external MSVC runtime imports and, on Windows, runs the extracted CLI's help.
GPU rendering requires a real graphics device and is not exercised by hosted
release CI. The ZIP contains unsigned executables; a checksum is not a signature.

## Optional HDR denoising

The native filter works without an additional runtime. On Windows, the optional
installer downloads the pinned official OIDN package and checks its SHA-256:

```text
python scripts/install-oidn.py
```

The runtime is placed in `runtime/oidn/` and stays outside version control.
Review the OIDN package's included notices if redistributing it with binaries.
`SYFRACK_OIDN` can point to a separately installed `oidnDenoise.exe`.
This option is CPU-side; it does not change the GPU's accumulated samples.

## Reproducibility

Record the scene, seed, render configuration, engine revision and GPU/driver
alongside an image. PNG, raw EXR and scene snapshots complement each other.
Checkpoint compatibility is tied to the renderer and shader implementation;
do not assume a checkpoint from another version can be resumed.

The opt-in `laptop` profile is intended for local tuning. A build made with
`-C target-cpu=native` may require the original CPU's instruction set and is
not the default portable build.
