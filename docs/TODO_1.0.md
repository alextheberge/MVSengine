# 1.x Readiness Roadmap

This document tracks the work required to move `mvs-manager` from the current feature-complete `0.x` phase into a stable `1.x` public contract.

## Current Readiness

- The scanner is now parser-backed across Rust, TypeScript/JavaScript, Go, Python, Java, Kotlin, C#, PHP, Ruby, Swift, Lua, and Luau.
- Manifests persist semantic evidence inventories, machine-readable command output, and per-language scan policy.
- The stable `1.x` manifest and command-output contract is now documented in `docs/CONTRACT_1X.md` and guarded by golden fixtures under `tests/fixtures/contracts/`.
- The main remaining work is release hardening, broader fixture depth, and public-release audit work.

## 1.0 Exit Criteria

- Stable manifest schema and scan-policy semantics are explicitly documented and versioned.
- Canonical signature formats are frozen for supported languages, with migration policy for any future changes.
- `generate`, `lint`, and `validate` have stable JSON output, stable exit behavior, and release-level fixture coverage.
- Workspace-aware export following is credible for the major ecosystems this tool claims to support.
- Real-world regression fixtures cover representative library, CLI, SDK, and plugin layouts.
- Install, release, and dogfood flows are reliable on supported platforms.

## P0: Must Finish Before 1.0

- Add release-grade regression fixtures.
  - Continue broadening multi-crate Rust workspace fixtures beyond the current direct and chained allowlisted member-facade coverage, including more mixed `pub mod` plus `pub use` layouts.
  - Continue broadening TS/JS fixtures beyond the current release-style workspace coverage for export maps, import maps, monorepo self-references, and path aliases.
  - Continue broadening Python facade and `__all__` fixtures beyond the current multiple-root coverage.
  - Continue broadening Ruby and Lua/Luau release fixtures beyond the current runtime-boundary coverage.
- Run a public-release audit.
  - Verify installer and release assets on macOS, Linux, and Windows.
  - Review onboarding docs for a first-time public user, not just dogfood usage.
  - Decide whether to ship `1.0.0-rc1` first or cut `1.0.0` directly. The repo now has a dedicated prerelease path for `-rcN` tags.

## P1: Strongly Recommended For 1.0

- Publish policy guidance and defaults.
  - Recommend conservative vs aggressive export-following presets per ecosystem.
  - Document when `public_api_roots`, include/exclude filters, and export-following should be used together.
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

- Expand fixture coverage beyond the current direct and chained allowlisted workspace-member facades, including more mixed `pub mod` plus `pub use` layouts.
- More real-layout workspace fixtures around private implementation modules and facade crates.

### TypeScript / JavaScript

- Release-style workspace coverage now exists; remaining fixture gaps are deeper nested packages and more mixed export-map styles.

### Python

- Dynamic export assembly still falls back by design.
- External-package wildcard imports should remain conservative unless policy explicitly broadens them.
- Multiple module roots are now covered; remaining fixture gaps are namespace-like layouts and more layered facade chains.

### Ruby

- Release-style export-boundary coverage now exists; remaining gaps are more metaprogramming-heavy patterns.
- Clear boundaries for what stays intentionally unsupported in `1.0`.

### Lua / Luau

- Release-style returned-root coverage now exists; remaining gaps are more module export tables assembled across multiple assignment styles.
- Explicit documentation for where runtime export heuristics stop.

## Testing And Verification

- Maintain green `cargo test`, `lint`, and `make ci` on every release candidate.
- Keep golden command and manifest fixtures current so canonical inventory or output changes are reviewed deliberately.
- Add integration tests for every persisted scan-policy mode.
- Track test counts and fixture categories in release notes so regressions are visible.

## Release Operations

- Verify installer and release assets on macOS, Linux, and Windows.
- Audit `README.md`, `docs/USAGE.md`, and `docs/RELEASE.md` before the first public release candidate.
- Keep `Cargo.toml`, `Cargo.lock`, and dogfood `mvs.json` versions aligned.
- Decide whether `1.0.0` requires a dedicated release candidate phase or can ship directly from `0.x` after checklist completion.

## Suggested Sequence

1. Deepen multi-workspace and real-layout release fixtures.
2. Add any missing golden fixtures for new contract surfaces before feature work resumes.
3. Run a public-release docs and installer audit.
4. Cut a `1.0.0-rc1` if any stability uncertainty remains; otherwise cut `1.0.0`.
