# Distribution options for `mvs-manager`

The primary supported install path today is **GitHub Releases** plus [`scripts/install.sh`](../scripts/install.sh) / [`scripts/install.ps1`](../scripts/install.ps1), with SHA-256 verification via `checksums.txt`.

## Evaluation matrix (first-party channels)

| Channel | Pros | Cons / notes |
|---------|------|----------------|
| **GitHub Releases + curl installer** (current) | Checksums, multi-target archives, no registry account required | Requires trusting the script host + release org; `self-update` shells out to the same installer. |
| **`cargo install` from crates.io** | Familiar to Rust developers; `cargo install --locked` pins deps | AGPL-3.0 affects downstream packaging expectations; crate name availability and release cadence must align with Git tags. |
| **Homebrew tap (third-party or official)** | Great macOS UX; `brew upgrade` | Requires maintaining a formula, bottles per OS, and review if submitted to `homebrew-core`. |
| **Nix flake** | Reproducible hashes; fits NixOS and devshells | Higher maintenance; users must use Nix. |
| **`cargo binstall`** | Fast binary install if published with metadata | Depends on publishing `CARGO_BINSTALL` metadata or hosting fallbacks. |

## Recommendation

1. Keep **GitHub Releases** as the canonical distribution and signing/checksum story (`make release-verify` in the Makefile).
2. Add **optional** `cargo install` publishing only if you want Rust-native discovery and accept the AGPL obligations for a crates.io artifact.
3. Add **Homebrew** or **Nix** when there is sustained demand from a specific community; both are thin wrappers around the same release binaries.

Forks should set `MVS_REPO` (or `MVS_UPDATE_REPO`) consistently for `install.sh`, `self-update`, and `mvs-manager doctor` output so all tooling points at the same `owner/name`.
