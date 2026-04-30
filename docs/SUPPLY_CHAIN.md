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
2. Downloads `checksums.txt` and the archive over HTTPS using **ureq** (Rustls TLS).
3. Parses the `sha256sum`-style line for the archive basename and compares it to an incremental SHA-256 of the downloaded archive bytes (**fail closed** on missing line or mismatch).
4. Extracts `mvs-manager` or `mvs-manager.exe` from the archive root (`tar` + `flate2` on Unix, `zip` on Windows).
5. Replaces the binary next to the current executable using a best-effort atomic pattern (platform-specific; Windows may require closing other instances if the exe is locked).

Optional `MVS_GITHUB_TOKEN` / `GITHUB_TOKEN` is sent on these GETs if set (same idea as other GitHub calls), but anonymous download usually works for public assets.

### What is not verified yet (phase 2)

Release signing (for example `checksums.txt.asc` / minisign or OpenPGP) is **not** checked by `self-update` today. The binary trusts any `checksums.txt` that passes TLS and matches the archive hash line. A separate release workflow can produce signatures; wiring verification into the CLI is a planned follow-up.

## Legacy escape hatch

If `MVS_LEGACY_SHELL_INSTALL` is set to `1`, `true`, `yes`, or `on`, `self-update` uses the previous behavior: remote install script via `curl | bash` (Unix) or `irm | iex` (Windows). This is **unsupported for production** and intended only for debugging or unusual environments.

## Related docs

- [DISTRIBUTION.md](DISTRIBUTION.md) — distribution channels
- [INSTALL_AND_CI.md](INSTALL_AND_CI.md) — CI env vars including `MVS_LEGACY_SHELL_INSTALL`
