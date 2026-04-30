# Pinning `mvs-manager` in repos and CI

This guide complements [USAGE.md](USAGE.md) and the install scripts under `scripts/`.

## Pinning by version (recommended)

Release assets follow this pattern (see GitHub Releases):

- `mvs-manager-<semver>-<target>.tar.gz` (or `.zip` on Windows)
- `checksums.txt` (SHA-256 for each archive)

Set `MVS_VERSION` (for example `v1.6.0`) and `MVS_REPO` when using [`scripts/install.sh`](../scripts/install.sh) so every machine and CI job installs the same build.

### mise / asdf-style `.tool-versions`

Example (adjust version and checksum URL to your release):

```text
# .tool-versions — use a custom plugin or a small wrapper; see below for curl pattern
mvs-manager 1.6.0
```

Because `mvs-manager` is not bundled as a core mise plugin, the portable pattern is a **one-line install step** in `mise.toml` `[tasks]` or in CI that runs `install.sh` with a pinned `MVS_VERSION`.

### GitHub Actions (pinned install)

Use a dedicated step with environment variables so forks can point at their own repo:

```yaml
env:
  MVS_NO_UPDATE_CHECK: "1"
  MVS_VERSION: "v1.6.0"
  # Optional: avoid anonymous GitHub API rate limits on busy org runners
  GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}

jobs:
  mvs:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install mvs-manager
        run: |
          curl -fsSL https://raw.githubusercontent.com/alextheberge/MVSengine/master/scripts/install.sh | bash
        env:
          MVS_INSTALL_DIR: ${{ github.workspace }}/.local/bin
          MVS_VERSION: ${{ env.MVS_VERSION }}
      - name: Lint manifest
        run: |
          echo "${{ github.workspace }}/.local/bin" >> "$GITHUB_PATH"
          mvs-manager lint --root . --manifest mvs.json --format json
```

The installer verifies `checksums.txt` for the downloaded archive.

## CI environment variables

| Variable | Purpose |
|----------|---------|
| `MVS_NO_UPDATE_CHECK` | Set to any non-empty value to disable interactive stderr “update available” hints (recommended in CI). |
| `GITHUB_TOKEN` or `MVS_GITHUB_TOKEN` | Authenticated GitHub API access for update checks or `self-update --check` if you hit rate limits. |
| `MVS_REPO` / `MVS_UPDATE_REPO` | `owner/name` for releases and raw install scripts (forks and private mirrors). Same semantics as `install.sh`. |
| `MVS_ALLOW_UNSAFE_SELF_UPDATE` | Set to `1`/`true`/`yes`/`on` only if you intentionally run `self-update` from a Cargo build directory or other non-standard location. |

## Diagnostics

Run `mvs-manager doctor --format json` in CI to dump resolved paths, tool availability, and update-related configuration when debugging install issues.
