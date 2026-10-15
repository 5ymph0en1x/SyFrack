# Local release verification

Prepared on 15 October 2026 for manual publication by the project owner.

The standalone export was checked and built locally. All checks below passed
against its source files; Cargo build artifacts were kept outside the export.

| Check | Result |
| --- | --- |
| English/French catalog and source audit | 967 complete messages; matching format fields |
| Localization audit regression tests | 2 passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo check --locked --workspace --all-targets` | Passed from the export |
| `cargo test --locked --workspace --lib` | 450 passed; 2 deliberately ignored |
| Core scene/graph integration tests | 32 passed; 1 asset-generation helper ignored |
| Studio and CLI development binaries | Built successfully from the export |
| CLI help and language option | Passed |
| Vulkan compute probe | Passed: 256 expected values |
| File inventory and documentation links | SHA-256, size and local links verified |
| Optimized Windows x64 release binaries | Built with the static C runtime in 11 min 34 s |
| Portable release archive | 488 files; 24.71 MiB; every hash and packaged documentation link checked |
| Extracted release CLI | Help smoke test passed; no external MSVC runtime imports |
| Release binary Vulkan probe | Passed: Quadro K3100M, 256 expected values |
| Release packaging/publication regression tests | 10 passed; remote calls mocked |
| GitHub workflow static validation | Both workflows passed actionlint 1.7.12 |

The library suite comprises 203 Studio, 48 colour, 106 core, 4 localization and
89 renderer tests. The 32 integration tests are additional to those counts;
the core unit suite is also rerun by Cargo's `--tests` command.

UI checks cover both workshops, language switching without scene/history or
generated-shader changes, decimal separators, detached-dialog identity and
portrait thumbnail bounds. Real egui/wgpu captures were reviewed in English
and French, including the English 2D colour controls and custom render settings.
The Studio illustration labels its viewport as a saved native render reference.

Local commands used the pinned Rust toolchain and `--offline --locked`, with
dependencies already cached. The original executable check used the development
profile. The additional release check used `--release --target
x86_64-pc-windows-msvc` and `RUSTFLAGS=-C target-feature=+crt-static`, without
`target-cpu=native`. Both executables passed PE architecture/subsystem and import
checks. The local package records an all-zero source revision because this
preparation has no publication commit yet; it is a packaging validation artifact,
kept outside the export, not an official GitHub release. Actions will substitute
the exact commit resolved from the owner's version tag.

The package includes the two optimized binaries, curated scenes/graphs/HDRI,
guides, the optional OIDN installer (not the OIDN runtime), dependency notices and
embedded-font license texts. A 217-package dependency inventory includes resolved
build/test dependencies as well as runtime dependencies. The outer ZIP checksum
and all inner file hashes were verified; the extracted CLI ran successfully.
The release-profile CPU test steps are configured in Actions; the 482 Rust tests
reported above were run with the development test profile during source
preparation, not rerun locally in the release profile for this packaging change.

The export inventory excludes its own manifest and is regenerated after the
final documentation/image update. The matching HDRI provenance checksums were
verified during preparation. No working renders or checkpoints were moved.

Both GitHub workflows are supplied but have not been run on GitHub by this preparation.
No repository, remote, commit, tag, release or upload has been created.

The source export contains no prebuilt Studio/CLI binaries. Windows x64 is the
locally tested target. Vulkan graphics checks use an NVIDIA Quadro K3100M;
results are not a claim of compatibility with every GPU or operating system.
The full set of ignored GPU integration tests was not rerun for this localization
change. The local compute probe and offscreen GPU UI captures are smoke checks.
