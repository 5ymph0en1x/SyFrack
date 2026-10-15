# Publishing this source tree

This folder is a standalone source export. No Git repository, remote, commit,
tag, release or upload was created by the preparation process.

1. Review `README.md`, `LICENSE`, `THIRD_PARTY_NOTICES.md` and the verification
   report in `docs/RELEASE_VERIFICATION.md`.
2. Run the commands in `docs/BUILDING.md` from this folder if you want to repeat
   the checks on another machine. Keep `Cargo.lock` in version control.
3. Create your GitHub repository, then upload **the contents of this folder**
   at its root, including `.github/`. `Source checks` runs on pushes and pull
   requests. Keep `SOURCE_MANIFEST.json` with the source files.
4. Publish a version tag as described below. The separate `Windows release`
   workflow builds and attaches downloadable Windows x64 binaries.
5. Describe the first release as experimental. Hosted checks do not validate
   every graphics device; only Windows x64 binaries are provided.

## Publish downloadable binaries

Commit and push the complete source tree first. The version in `Cargo.toml`
must match the tag (for example `0.1.0` and `v0.1.0`). From your Git checkout:

```text
git tag -a v0.1.0 -m "SyFrack Studio 0.1.0 - experimental release"
git push origin v0.1.0
```

Pushing a `v*` tag starts **Actions → Windows release** automatically. It checks
translations and formatting, runs CPU/headless UI and core integration tests
with the release profile, builds Studio and CLI, then verifies the portable ZIP.
After a successful build, it creates a **draft** GitHub Release, uploads the ZIP
and `SHA256SUMS.txt`, then publishes it. Tags such as `v0.2.0-beta.1` create a
prerelease (set Cargo's version to `0.2.0-beta.1` first).

The download assets are:

- `SyFrack-v0.1.0-windows-x64.zip`: both executables and the required content.
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
directory. Curated source scenes and the required regression fixtures remain.
The existing workbench and its render files are not moved or deleted.

From the working checkout, `python scripts/package/prepare-source.py` regenerates
the managed export. It refuses to overwrite a destination file that has been
changed independently since the previous export. It never uploads anything.

If you edit the exported README or images directly, preserve those edits in the
working source before regenerating. The inventory covers the current JPEG
showcase and the three Studio screenshots; it does not include itself.
