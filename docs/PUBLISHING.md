# Publishing this source tree

This folder is a standalone source export. No Git repository, remote, commit,
tag, release or upload was created by the preparation process.

1. Review `README.md`, `LICENSE`, `THIRD_PARTY_NOTICES.md` and the verification
   report in `docs/RELEASE_VERIFICATION.md`.
2. Run the commands in `docs/BUILDING.md` from this folder if you want to repeat
   the checks on another machine. Keep `Cargo.lock` in version control.
3. Update your GitHub checkout with **the contents of this folder** at its root,
   including `.github/`, and review deleted files as well as additions. Keep
   the checkout's existing `.git/` directory and history. `Source checks` runs on pushes and pull
   requests. Keep `SOURCE_MANIFEST.json` with the source files.
4. For a new binary release, publish a new version tag as described below. The separate `Windows release`
   workflow builds and attaches downloadable Windows x64 binaries.
5. Describe this release as experimental. Hosted checks do not validate
   every graphics device; only Windows x64 binaries are provided.

## Publish downloadable binaries

Commit and push the complete source tree first. The version in `Cargo.toml`
must match the tag (for example `0.2.0` and `v0.2.0`). From your Git checkout:

```text
git tag -a v0.2.0 -m "SyFrack Studio 0.2.0 - experimental release"
git push origin v0.2.0
```

Pushing a `v*` tag starts **Actions → Windows release** automatically. It checks
translations and formatting, runs CPU/headless UI, core and CLI integration
tests with the release profile, builds Studio and CLI, then verifies the portable ZIP.
After a successful build, it creates a **draft** GitHub Release, uploads the ZIP
and `SHA256SUMS.txt`, then publishes it. Tags such as `v0.2.0-beta.1` create a
prerelease (set Cargo's version to `0.2.0-beta.1` first).

The download assets are:

- `SyFrack-v0.2.0-windows-x64.zip`: both executables and the required content.
- `SHA256SUMS.txt`: SHA-256 of that ZIP.

Users download these under **Releases → Assets**, extract the entire ZIP and
launch `syfrack-studio.exe`. They do not need Rust or Python. A compatible GPU
driver remains necessary. These experimental executables are unsigned.

You can also run **Actions → Windows release → Run workflow** and enter an
**existing** version tag. The workflow must be on the default branch for this
button to appear. Use this to retry a failed build/publication. An existing
draft can be completed; an already published release is preserved and the run
fails rather than replacing its assets. Use a new version for a new build.
Do not publish an empty Release manually before this workflow finishes.

Build artifacts are also retained under the Actions run for 14 days. Release
downloads remain attached to the Release independently of that retention.
The workflow uses the automatic `GITHUB_TOKEN`; no personal access token is
needed. Only the publication job requests `contents: write`. Repository or
organization policy must allow GitHub Actions and this permission.

The remote tag is checked again before publication to ensure it still points
to the compiled commit. This draft/upload/publish order also works with
[immutable releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases).
Publication uses the [official GitHub CLI](https://cli.github.com/manual/gh_release).

`SOURCE_MANIFEST.json` records SHA-256 and size for the prepared files. It is
an integrity inventory, not a digital signature or proof of authorship.
Exported text uses LF line endings, matching `.gitattributes`, so Git does not
change those bytes when checking out the published tree.

The export excludes render/checkpoint folders, build caches, downloaded OIDN
binaries, local shortcuts, editor/agent state, archives and the original `.git`
directory. Editable scene sources, including the small study scenes used by
examples, and the required regression fixtures remain. Documentation includes
its linked guides, figures and benchmark evidence; binaries use the same closure.
The three EXR files under `crates/syfrack-render/tests/fixtures/parity/` are
historical regression inputs. They are explicitly included and exempted from
the general render-output ignore rule; working EXR renders remain excluded.
The three scene/reference pairs and synthetic studio environment for
`guiding_profile` are also included under
`docs/benchmarks/guiding-cost-2026-10-09/fixtures/`. The native HDR environment
`scenes/environments/neon-night.exr`, used by the cathedral scene, is another
explicit source asset. These assets are retained by the ignore rules and verifier.
The existing workbench and its render files are not moved or deleted.

From the working checkout, `python scripts/package/prepare-source.py` regenerates
the managed export. It refuses to overwrite a destination file that has been
changed independently since the previous export. It never uploads anything.
Unchanged files recorded in the old inventory are removed when they are no
longer selected. Independently changed or unmanaged files stop synchronization
before copying or deleting anything. `verify-source.py` validates the complete
inventory, local documentation links and the absence of build/render artifacts.

If you edit the exported README or images directly, preserve those edits in the
working source before regenerating. The inventory covers the showcase, current
Simple/Advanced and render screenshots, and native references used by the capture
example; it does not include itself. Run the Python checks with
`PYTHONDONTWRITEBYTECODE=1` and keep `CARGO_TARGET_DIR` outside the source export
so verification does not add caches to the publication folder.

The 22 October update is version **0.2.0**, recorded in `CHANGELOG.md`.
Preparing or pushing these source changes does not replace an existing version
tag or published release. After committing the reviewed tree, publish its new `v0.2.0` tag; preserve
earlier tags and release assets.
