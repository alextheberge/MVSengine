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
- Manual dispatch from Actions UI

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
5. Publishes release assets on tag builds.

## Signature verification (optional)

If release includes `checksums.txt.asc` and you have the public key:

```bash
scripts/verify-release.sh <archive> checksums.txt checksums.txt.asc <public-key.asc>
```

Without signature files, checksum verification still works.
