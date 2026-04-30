# Supply chain: installs and `self-update`

This document describes what the `mvs-manager` binary verifies today, what first-time installers still trust, and optional hardening.

## First-time install (`install.sh` / `install.ps1`)

The curl / PowerShell one-liners fetch a **remote script** from `raw.githubusercontent.com`, then run it. That script downloads the release archive and `checksums.txt`, verifies SHA-256, and extracts the binary. Trust includes:

- TLS to the script host and to `github.com` / release asset URLs
- The script’s contents at fetch time
- The `checksums.txt` line for the selected archive basename

## `mvs-manager self-update` (default)

By default, `self-update` does **not** spawn `bash`, `sh`, or `powershell` to interpret a remote install script. The running binary:

1. Resolves the same release layout as `scripts/install.sh` (tag, host triple, archive basename).
2. Downloads `checksums.txt` and the archive over HTTPS using **ureq** (Rustls TLS), with connect/read timeouts, optional `HTTPS_PROXY`/`HTTP_PROXY` via the client’s environment handling, and hard caps on download size (small cap for checksums/signature files, larger cap for archives).
3. If the release ships **`checksums.txt.minisig`**, verifies it with **minisign** before trusting the checksum list. The public key is read from [`packaging/minisign.pub`](../packaging/minisign.pub) baked into the build, or overridden at runtime with **`MVS_MINISIGN_PUBLIC_KEY`** (full minisign public key file contents, two lines). If a signature file is present but no key is configured, installation fails closed. **Release maintainers** must replace that file with the real signing public key before publishing releases that attach `checksums.txt.minisig` to GitHub; the key in the repository today matches the self-test fixture under `tests/fixtures/install_release/`.
4. Parses the `sha256sum`-style line for the archive basename and compares it to an incremental SHA-256 of the downloaded archive bytes (**fail closed** on missing line or mismatch).
5. Extracts `mvs-manager` or `mvs-manager.exe` from the archive root only when the member path is a single safe path segment (rejects `..`, absolute paths, symlinks masquerading as the binary, and duplicate matching entries).
6. Replaces the binary next to the current executable using a best-effort atomic pattern (platform-specific; Windows may require closing other instances if the exe is locked).

Optional `MVS_GITHUB_TOKEN` / `GITHUB_TOKEN` is sent on these GETs if set (same idea as other GitHub calls), but anonymous download usually works for public assets.

### GitHub API traffic (update checks)

`mvs-manager` uses the same **ureq** stack for `https://api.github.com/.../releases/latest` JSON (no `curl` / `wget` / PowerShell for the default path).

### Dependency policy (maintainers)

[`deny.toml`](../deny.toml) is enforced in CI with **`cargo deny check`** (Embark `cargo-deny-action`). It tracks allowed SPDX licenses (including **`CDLA-Permissive-2.0`** for `webpki-roots` pulled in by Rustls) and warns on duplicate crate versions where practical.

### Optional fuzzing

A **`checksum_parser`** libFuzzer target lives under [`fuzz/`](../fuzz/). Run locally with nightly Rust and `cargo install cargo-fuzz`, then `cd fuzz && cargo fuzz run checksum_parser`. A manual GitHub Actions workflow **Fuzz** runs a short bounded session on demand.

## Legacy escape hatch

If `MVS_LEGACY_SHELL_INSTALL` is set to `1`, `true`, `yes`, or `on`, `self-update` uses the previous behavior: remote install script via `curl | bash` (Unix) or `irm | iex` (Windows). This is **unsupported for production** and intended only for debugging or unusual environments.

## Related docs

- [DISTRIBUTION.md](DISTRIBUTION.md) — distribution channels
- [INSTALL_AND_CI.md](INSTALL_AND_CI.md) — CI env vars including `MVS_LEGACY_SHELL_INSTALL`
