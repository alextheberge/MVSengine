# 1.x Release Status

This document now records that the repository has been brought to its initial stable `1.x` cut state and tracks the follow-on backlog after that release line is established.

## Current Status

- The scanner is now parser-backed across Rust, TypeScript/JavaScript, Go, Python, Java, Kotlin, C#, PHP, Ruby, Swift, Lua, and Luau.
- Manifests persist semantic evidence inventories, machine-readable command output, and per-language scan policy.
- The stable `1.x` manifest and command-output contract is now documented in `docs/CONTRACT_1X.md` and guarded by golden fixtures under `tests/fixtures/contracts/`.
- Release-grade workspace and convention-boundary fixtures now exist for Rust, TypeScript/JavaScript, Python, Ruby, and Lua/Luau.
- Final and prerelease release paths are both wired: canonical releases use `make release-github`, and prereleases use `make release-rc RELEASE_TAG_SUFFIX=rcN`.
- `cargo test`, `cargo run -- lint --root . --manifest mvs.json`, `make ci`, and local package verification are all green at the `1.x` cut state.

## 1.x Criteria

- Stable manifest schema and scan-policy semantics are explicitly documented and versioned.
- Canonical signature formats are frozen for supported languages, with migration policy for any future changes.
- `generate`, `lint`, and `validate` have stable JSON output, stable exit behavior, and release-level fixture coverage.
- Workspace-aware export following is credible for the major ecosystems this tool claims to support.
- Real-world regression fixtures cover representative library, CLI, SDK, and plugin layouts.
- Install, release, and dogfood flows are reliable on supported platforms.

All of the above are now satisfied in-repo.

## Post-1.x Priorities

- Publish policy guidance and defaults.
  - Recommend conservative vs aggressive export-following presets per ecosystem.
  - Document when `public_api_roots`, include/exclude filters, and export-following should be used together.
- Improve public API reporting.
  - Extend boundary debugging to scan-path exclusions if public users need that level of traceability.
  - Decide whether `report` should also trace path-level scan exclusions and default ignored directories.
- Harden docs for public users.
  - Keep README focused on onboarding and link deeper docs instead of growing it indefinitely.
  - Add an end-to-end “library release” example and a “CLI release” example.

## Early 1.x Candidates

- Add policy presets such as `library`, `cli`, `plugin-host`, and `sdk`.
- Expand export-boundary parity further where behavior is still heuristic by design.
- Add package-manager and CI examples beyond the current dogfood flow.
- Add a compatibility-report command or richer JSON diff mode for bots and release automation.
- Add larger fixture corpora from real open-source layouts once licensing and maintenance are clear.

## Language-Specific Gap List

### Rust

- More mixed `pub mod` plus `pub use` layouts and larger real-layout workspace fixture sets.

### TypeScript / JavaScript

- Release-style workspace coverage now exists; remaining fixture gaps are deeper nested packages and more mixed export-map styles.

### Python

- Dynamic export assembly still falls back by design.
- External-package wildcard imports should remain conservative unless policy explicitly broadens them.
- Multiple module roots are now covered; remaining fixture gaps are namespace-like layouts and more layered facade chains.

### Ruby

- Release-style export-boundary coverage now exists; remaining gaps are more metaprogramming-heavy patterns.
- Clear boundaries for what stays intentionally unsupported in `1.x`.

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
- Audit `README.md`, `docs/USAGE.md`, and `docs/RELEASE.md` before major release transitions.
- Keep `Cargo.toml`, `Cargo.lock`, and dogfood `mvs.json` versions aligned.
- Use `make release-rc RELEASE_TAG_SUFFIX=rcN` for prereleases and `make release-github` for canonical finals.

## Suggested Sequence

1. Deepen fixture breadth only when a new contract surface or parser rule is added.
2. Keep golden fixtures current whenever manifest or JSON contracts evolve.
3. Prefer prerelease tags for higher-risk contract changes before the next stable release.
4. Treat the remaining items here as post-`1.x` refinement, not release blockers.
