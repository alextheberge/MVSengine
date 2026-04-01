# 1.x Readiness Roadmap

This document tracks the work required to move `mvs-manager` from the current feature-complete `0.x` phase into a stable `1.x` public contract.

## Current Readiness

- The scanner is now parser-backed across Rust, TypeScript/JavaScript, Go, Python, Java, Kotlin, C#, PHP, Ruby, Swift, Lua, and Luau.
- Manifests persist semantic evidence inventories, machine-readable command output, and per-language scan policy.
- The main remaining work is no longer raw language coverage. It is contract stability, machine-readable compatibility semantics, and release hardening.

## 1.0 Exit Criteria

- Stable manifest schema and scan-policy semantics are explicitly documented and versioned.
- Canonical signature formats are frozen for supported languages, with migration policy for any future changes.
- `generate`, `lint`, and `validate` have stable JSON output, stable exit behavior, and release-level fixture coverage.
- Workspace-aware export following is credible for the major ecosystems this tool claims to support.
- Real-world regression fixtures cover representative library, CLI, SDK, and plugin layouts.
- Install, release, and dogfood flows are reliable on supported platforms.

## P0: Must Finish Before 1.0

- Freeze the compatibility contract.
  - Define which `mvs.json` fields are stable in `1.x`.
  - Define which `scan_policy` options are stable in `1.x`.
  - Document compatibility guarantees for canonical signature inventories and JSON command output.
- Tighten validation semantics.
  - Make `validate` return machine-usable failing axes, not only free-text reasons.
  - Publish a stable exit-code matrix for success, degraded compatibility, incompatibility, invalid manifest, and internal failure.
- Add release-grade regression fixtures.
  - Broaden multi-crate Rust workspace fixtures beyond the current allowlisted member-reexport path.
  - TS/JS package-export, import-map, monorepo self-reference, and path-alias fixtures.
  - Python facade and `__all__` fixtures across multiple roots.
  - Convention-driven Ruby and Lua/Luau export-boundary fixtures.

## P1: Strongly Recommended For 1.0

- Publish policy guidance and defaults.
  - Recommend conservative vs aggressive export-following presets per ecosystem.
  - Document when `public_api_roots`, include/exclude filters, and export-following should be used together.
- Strengthen output contracts.
  - Add schema docs for JSON output and include field-level examples.
  - Add changelog guidance for any future output expansion without breaking automation.
- Improve public API reporting.
  - Surface concrete added/removed inventory entries in `lint` and `validate` JSON.
  - Show which scan-policy rule included or excluded a declaration when debugging a boundary.
- Harden docs for public users.
  - Keep README focused on onboarding and link deeper docs instead of growing it indefinitely.
  - Add an end-to-end “library release” example and a “CLI release” example.

## P2: Good Candidates For Early 1.x

- Add policy presets such as `library`, `cli`, `plugin-host`, and `sdk`.
- Expand export-boundary parity further where behavior is still heuristic by design.
- Add package-manager and CI examples beyond the current dogfood flow.
- Add a compatibility-report command or richer JSON diff mode for bots and release automation.
- Add larger fixture corpora from real open-source layouts once licensing and maintenance are clear.

## Language-Specific Gap List

### Rust

- Expand fixture coverage for allowlisted workspace-member reexports and deeper facade stacks.
- Stronger handling for facade crates that reexport private internals through multiple layers.

### TypeScript / JavaScript

- Broaden monorepo fixture coverage around nested packages and mixed export-map styles.

### Python

- Dynamic export assembly still falls back by design.
- External-package wildcard imports should remain conservative unless policy explicitly broadens them.
- More fixture coverage for nonstandard roots and namespace-like layouts.

### Ruby

- Additional coverage for more metaprogramming-heavy export patterns.
- Clear boundaries for what stays intentionally unsupported in `1.0`.

### Lua / Luau

- More module export fixtures for tables assembled across multiple assignment styles.
- Explicit documentation for where runtime export heuristics stop.

## Testing And Verification

- Maintain green `cargo test`, `lint`, and `make ci` on every release candidate.
- Add golden manifest fixtures so canonical inventory changes are reviewed deliberately.
- Add integration tests for every persisted scan-policy mode.
- Track test counts and fixture categories in release notes so regressions are visible.

## Release Operations

- Verify installer and release assets on macOS, Linux, and Windows.
- Audit `README.md`, `docs/USAGE.md`, and `docs/RELEASE.md` before the first public release candidate.
- Keep `Cargo.toml`, `Cargo.lock`, and dogfood `mvs.json` versions aligned.
- Decide whether `1.0.0` requires a dedicated release candidate phase or can ship directly from `0.x` after checklist completion.

## Suggested Sequence

1. Deepen multi-workspace fixtures and golden manifests.
2. Freeze and document JSON output, manifest schema, and scan-policy stability.
3. Add release-grade multi-workspace fixtures and golden manifests.
4. Tighten `validate` into a stronger machine-readable compatibility gate.
5. Run a public-release docs and installer audit.
6. Cut a `1.0.0-rc1` if any stability uncertainty remains; otherwise cut `1.0.0`.
