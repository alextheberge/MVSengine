# Distribution options for `mvs-manager`

The primary supported install path today is **GitHub Releases** plus [`scripts/install.sh`](../scripts/install.sh) / [`scripts/install.ps1`](../scripts/install.ps1), with SHA-256 verification via `checksums.txt`.

## Evaluation matrix (first-party channels)

| Channel | Pros | Cons / notes |
|---------|------|----------------|
| **GitHub Releases + curl installer** (current) | Checksums, multi-target archives, no registry account required | First-time install still trusts the script host + release org; `self-update` uses the same release bytes as `install.sh` but fetches and verifies them in-process (see [SUPPLY_CHAIN.md](SUPPLY_CHAIN.md)). |
| **`cargo install` from crates.io** | Familiar to Rust developers; `cargo install --locked` pins deps | AGPL-3.0 affects downstream packaging expectations; crate name availability and release cadence must align with Git tags. |
| **npm `mvs-manager` package** ([`packages/mvs-manager`](../packages/mvs-manager)) | Familiar to JS/TS repos as a `devDependency`; downloads the same release binary | Requires Node ≥18 for postinstall; still AGPL for the CLI itself. |
| **`cargo binstall`** | Fast binary install (skips compiling), reuses release archives | Metadata is in place ([`Cargo.toml`](../Cargo.toml)'s `[package.metadata.binstall]`), but unverified against a real release until one is cut and `cargo binstall --dry-run mvs-manager` is run against it. |
| **Docker image** ([`Dockerfile`](../Dockerfile)) | No local Rust toolchain needed; pinned, reproducible runtime | Multi-stage build written and reviewed, but not build-tested here (no Docker daemon in this environment) — run `docker build -t mvs-manager .` once before trusting it, and publish to a registry (e.g. `ghcr.io/alextheberge/mvs-manager`) only with the account owner's say-so. |
| **Nix flake** ([`flake.nix`](../flake.nix)) | Reproducible hashes; fits NixOS and devshells | Written using the standard `buildRustPackage` pattern, but not build-tested here (no `nix` binary in this environment) — run `nix build` / `nix flake check` before relying on it. |
| **Homebrew tap** ([`packaging/homebrew/mvs-manager.rb`](../packaging/homebrew/mvs-manager.rb)) | Great macOS UX; `brew upgrade` | Template only — checksums are placeholders until filled in from a real release's `checksums.txt`; not submitted to any tap. See [`packaging/README.md`](../packaging/README.md). |
| **Scoop bucket** ([`packaging/scoop/mvs-manager.json`](../packaging/scoop/mvs-manager.json)) | Simple Windows CLI UX | Template only, same caveats as Homebrew above. |
| **winget** ([`packaging/winget/`](../packaging/winget)) | Built into Windows 11 | Template only (3-file manifest format), same caveats as Homebrew above; needs `winget validate` against a real release before submission. |
| **WASM / browser** ([`crates/mvs-wasm`](../crates/mvs-wasm)) | Runs the version-scheme translator client-side, no install at all | Only exposes `convert-version`'s logic today (not the full CLI: no filesystem/crawler access in-browser); see the [interactive playground](#interactive-playground) below. |

## Recommendation

1. Keep **GitHub Releases** as the canonical distribution and signing/checksum story (`make release-verify` in the Makefile).
2. Prefer the composite **GitHub Action** ([`.github/actions/setup-mvs`](../.github/actions/setup-mvs)) or the **npm wrapper** for pin-friendly CI/devDependency installs.
3. Add **optional** `cargo install` publishing only if you want Rust-native discovery and accept the AGPL obligations for a crates.io artifact.
4. Cut the templates in **Homebrew/Scoop/winget** into real submissions, and build-test **Docker**/**Nix**, once there is sustained demand and a maintainer available to keep them current — none of the four has been published anywhere yet.

## Crate layout (v3.0+)

The Rust code is a Cargo workspace of three crates, split so the versioning
logic is embeddable without pulling in the CLI or the tree-sitter-heavy
crawler:

- **`mvs-core`** — manifest model, hashing, the version-scheme translators, and version-source read/write. No network or process dependencies; this is what `mvs-wasm` binds.
- **`mvs-crawler`** — the tree-sitter-backed public-API/protocol-surface scanners for all supported languages.
- **`mvs-manager`** (root package) — the CLI, depending on both of the above.
- **`mvs-wasm`** — `wasm-bindgen` bindings over `mvs-core`'s `convert-version` logic, built with `wasm-pack build --target web`.

`napi-rs` (Node native addon) and `pyo3` (Python wheel) bindings over
`mvs-core` are planned but **not yet built** — `mvs-wasm` is the only binding
shipped so far. Fold the `convert_version_inner`-style thin-FFI-boundary
pattern from `crates/mvs-wasm/src/lib.rs` into a napi-rs/pyo3 crate when that
work happens; both `mvs-core` functions are already pure `Result<T, String>`
so the boundary work is the only remaining piece.

## Interactive playground

A live, WASM-backed demo of `convert-version` (paste a legacy version string,
pick a scheme, see the MVS mapping) is published at:
<https://claude.ai/artifact/XWg1XtbGqvtNXSwQSFHeaF>

It's a standalone static page (not hosted on this repo's infrastructure) —
see `mvs.dev` below for what a first-party hosted version would need.

## `mvs.dev`

`mvs.json`'s `$schema` field points at `https://mvs.dev/schema/v2`, and the
adoption plan calls for hosting an interactive migration playground there.
Neither is live: DNS/hosting for `mvs.dev` is outside what this repository
can provision on its own — it needs a domain owner to register it and point
it at a static host (e.g. the built `crates/mvs-wasm/pkg/` output plus a copy
of the playground HTML above, or GitHub Pages).

Forks should set `MVS_REPO` (or `MVS_UPDATE_REPO`) consistently for `install.sh`, `self-update`, and `mvs-manager doctor` output so all tooling points at the same `owner/name`.

Upstream CI runs the full Rust gate on **Linux, macOS, and Windows** so release-style paths stay healthy across platforms (see [INSTALL_AND_CI.md](INSTALL_AND_CI.md)).
