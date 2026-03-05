# Release Playbook

## Local packaging (single target)

```bash
make release-host
```

Or explicit target:

```bash
make release-target TARGET=x86_64-unknown-linux-gnu
```

Artifacts are written to `dist/<version_tag>/`.

## What gets produced
- `mvs-manager-<version>-<target>.<tar.gz|zip>`
- `*.sha256` per archive
- `checksums.txt` aggregate checksum file

## GitHub release pipeline

Workflow: `.github/workflows/release.yml`

Trigger options:
- Push a tag: `vX.Y.Z`
- Push changes to `mvs.json` or `Cargo.toml` on `main`/`master` (auto-tags canonical version and dispatches release)
- Manually run `Auto Tag Version` from Actions UI (`Run workflow`) to force tag/release dispatch
- Manual dispatch from Actions UI

Before release:

```bash
make dogfood-sync-version
make ci
```

Automated equivalent (sync + CI + commit version files + push branch):

```bash
make release-github
```

`release-github` pushes your current branch (default `origin`) so `Auto Tag Version` can create the canonical `vX.Y.Z` tag and dispatch `Release`.

This enforces:
- `Cargo.toml` version = MVS numeric version (`ARCH.FEAT.PROT`)
- canonical release tag = `vARCH.FEAT.PROT` (for example `mvs.json: 0.2.3-cli` => tag `v0.2.3`)

Build matrix currently publishes:
- `x86_64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

The workflow:
1. Builds each target in a dedicated job.
2. Packages archive + checksum per target.
3. Merges `checksums.txt`.
4. Optionally signs checksums with GPG when `MVS_GPG_PRIVATE_KEY` secret is configured.
5. Publishes or updates release assets for tag/dispatch events.

## Signature verification (optional)

If release includes `checksums.txt.asc` and you have the public key:

```bash
scripts/verify-release.sh <archive> checksums.txt checksums.txt.asc <public-key.asc>
```

Without signature files, checksum verification still works.
