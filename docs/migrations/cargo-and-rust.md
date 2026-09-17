# Migrating a Cargo/Rust project to MVS

Applies to a single crate or a Cargo workspace (including the
root-package-as-workspace-root layout this repo itself uses — see
`Cargo.toml` for a worked example: a `[workspace]` table and a `[package]`
table in the same file).

## What `mvs-manager` detects

- `Cargo.toml#package.version`, including `version.workspace = true`
  inheritance and a `[workspace.package].version` fallback.
- Scheme is `semver` (crates.io enforces SemVer), default mapping
  `MAJOR->ARCH, MINOR->FEAT, PATCH->FIX`.
- Public API surface: AST-canonicalized Rust signatures
  (`rust:fn run() -> i32`, `rust:impl-fn Type::method(...)`).
  `scan_policy.rust_export_following: public_modules` expands a rooted file
  like `src/lib.rs` across same-crate `pub mod`/`pub use` graphs (including
  glob and chained re-exports) so the contract surface matches what
  downstream crates can actually import; `scan_policy.rust_workspace_members`
  restricts that expansion to specific member crates when a facade
  intentionally re-exports only some of them.

## Steps

```bash
mvs-manager migrate detect --root .
mvs-manager migrate plan --root .
mvs-manager migrate backfill --tags v0.1.0..HEAD   # optional
mvs-manager migrate apply --root .
mvs-manager sync --check   # add to CI
```

## Workspace split note

If you split a crate into a Cargo workspace after adopting MVS (the way this
repository split `mvs-manager` into `mvs-core`/`mvs-crawler`/`mvs-manager` in
its own `v3.0`), re-run `mvs-manager lint` and compare the
`public_api_inventory` before and after: a pure `git mv` + re-export split
(`pub use new_crate::{a, b, c};` from the old module path) should produce
**zero** inventory drift, since the public surface hasn't actually moved for
external callers. If the crawler's candidate count changes because it now
also walks the new crate directories, add the new top-level directory to
`scan_policy.exclude_paths` only if those directories aren't meant to expose
their own public API to consumers of the split-off crates.

## crates.io / AGPL note

Publishing an AGPL-3.0-licensed crate to crates.io is allowed, but downstream
consumers inherit AGPL obligations — see this repo's own
[SUPPLY_CHAIN.md](../SUPPLY_CHAIN.md) and license header for the pattern this
project uses. `cargo binstall` metadata (`[package.metadata.binstall]` in
`Cargo.toml`) lets `cargo binstall <name>` fetch a prebuilt release archive
instead of compiling from source, independent of whether the crate itself is
published to crates.io.
