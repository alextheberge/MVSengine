# MVS Engine - Action Plan

## Phase 0 - Project Foundation
- [x] Finalize language/runtime choice (Rust single-binary target).
- [x] Establish repository layout for `mvs-manager` CLI and domain modules.
- [x] Add baseline `mvs.json` so the tool self-describes via MVS.
- [x] Add development workflow checks (`fmt`, `check`, `test`) to CI.

### Exit Criteria
- Repository builds locally with one command.
- `mvs.json` exists and matches schema baseline.

## Phase 1 - Manifest Contract and Core Types
- [x] Implement `mvs.json` typed schema model in code.
- [x] Add strict validation for identity format: `ARCH.FEAT.PROT-CONT`.
- [x] Implement compatibility structures: host/extension protocol ranges + legacy shims.
- [x] Implement AI contract structures and defaults.
- [x] Implement deterministic hash/evidence helpers.

### Exit Criteria
- Manifest can be loaded, validated, and written with stable formatting.
- Invalid schema content is rejected with clear error messages.

## Phase 2 - Generator (Crawler + Version Derivation)
- [x] Implement regex crawler for `@mvs-feature` and `@mvs-protocol` tags.
- [x] Implement public API signature extraction across core languages.
- [x] Compute evidence hashes (`feature_hash`, `protocol_hash`, `public_api_hash`).
- [x] Compare with prior manifest to derive `FEAT`/`PROT` increments.
- [x] Support manual `ARCH` bump flag for data/schema/system-generation breaks.
- [x] Write updated `mvs.json` and print human-readable reasoning.

### Exit Criteria
- Running generator on a code change updates manifest deterministically.
- CLI explains why each axis changed and points to relevant file(s).

## Phase 3 - Linter (Build-Gating)
- [x] Validate codebase evidence against `mvs.json` evidence.
- [x] Fail on public API drift without matching `PROT` update flow.
- [x] Fail on AI tool schema drift without matching `PROT` update flow.
- [x] Add readable diagnostics with actionable remediation steps.

### Exit Criteria
- Linter exits non-zero on any protocol/AI contract mismatch.
- Error output states exact reason and likely source file(s).

## Phase 4 - Reader/Validator (Host <-> Extension Compatibility)
- [x] Implement protocol range checks (host and extension perspectives).
- [x] Implement capability requirement checks.
- [x] Implement degraded-mode logic via `legacy_shims`.
- [x] Emit compatibility decision report (`compatible`, `degraded`, `reasons`).

### Exit Criteria
- Reader can load two manifests and return deterministic compatibility decisions.
- Out-of-range protocol can pass only when shim rules allow it.

## Phase 5 - Tests and Fixtures
- [x] Create fixture projects for multi-language scanning.
- [x] Add unit tests for manifest parsing/validation.
- [x] Add unit tests for crawler extraction and hash stability.
- [x] Add integration tests for generator/linter/reader commands.
- [x] Add regression cases for double-channel breaks (`ARCH` + `PROT`).

### Exit Criteria
- Core test suite covers success + failure paths for all modules.
- Version-axis decisions are reproducible and documented.

## Phase 6 - Packaging and Distribution
- [x] Produce release binaries for macOS/Linux/Windows.
- [x] Add install methods and checksum/signature verification.
- [x] Publish usage docs with examples and troubleshooting.

### Exit Criteria
- Portable binaries are generated in release pipeline.
- Documentation covers end-to-end workflows.

## Immediate Next Sprint
- [x] Scaffold CLI commands: `generate`, `lint`, `validate`.
- [x] Add baseline domain modules (`manifest`, `crawler`, `reader`).
- [x] Generate initial `mvs.json` for dogfooding.
- [x] Pass `cargo fmt` and `cargo check`.

---

## Phase 7 - Developer Experience & Onboarding

### High Priority
- [x] **`mvs init` command**: Detect project language from filesystem (Cargo.toml → Rust, package.json → TS/JS, go.mod → Go, etc.), generate a starter `mvs.json` with sensible scan_policy defaults. Accept `--context`, `--root`, `--force`, `--dry-run`, `--preset` flags. (`src/commands/init.rs`)
- [x] **GitHub Actions annotations**: When `GITHUB_ACTIONS=true` is set in the environment, emit `::error::` and `::warning::` annotation lines from `lint` so failures appear inline on PR diffs rather than as a raw exit code. (`src/commands/linter.rs`)
- [x] **`mvs schema` command**: Output the canonical JSON Schema for `mvs.json` to stdout (or write it to a file with `--output`). Enables `$schema`-based editor autocompletion and external validation. (`src/commands/schema.rs`)
- [x] **`--explain` flag on `lint`**: Augment lint failure output with per-failure remediation steps, specific drifted symbol names and their source files, and the exact `mvs-manager generate` invocation needed to resolve each issue. (`src/commands/linter.rs`)
- [x] **`mvs validate-all` command**: Accept a directory or explicit list of `mvs.json` files and run host/extension matrix validation across all pairs, emitting a structured compatibility report. Closes the monorepo batch-validation gap. (`src/commands/validate_all.rs`)

### Medium Priority
- [ ] **Scan policy presets**: Add named presets (`library`, `cli`, `plugin`, `sdk`) that configure sensible `scan_policy` defaults. Usable via `mvs init --preset library` or as a `scan_policy.preset` field in `mvs.json`.
- [x] **`mvs watch` command**: Re-run maintenance on a cadence using workspace fingerprinting, with optional `--remediate` for auto-generating manifest updates and `--once` for scheduler-friendly single-pass runs. (`src/commands/watch.rs`)
- [x] **`--remediate` flag on `lint`**: Auto-run `generate` when lint detects drift, then re-lint. (`src/commands/linter.rs`)
- [ ] **`report` command `--with-source-context` flag**: Re-crawl both manifests' source trees and attach `boundary_debug` information to the diff, showing which code change triggered each manifest delta.
- [x] **Manifest self-validation command (`mvs check-manifest`)**: Validates schema field, identity string consistency, range inversion, shim integrity, missing API root files, and stale evidence hashes. (`src/commands/check_manifest.rs`)
- [x] **Version constraint authoring helper (`mvs constraint`)**: Given two manifests, computes and prints the tightest valid `extension_range`/`host_range` pair, with optional `--lookahead N` to widen. (`src/commands/constraint.rs`)

### Low Priority
- [ ] **Dart adapter module**: Refactor the inlined Dart extraction (~200 lines in `crawler.rs`) into `src/mvs/crawler/adapters/dart.rs` following the same module pattern as other languages. Create a `DartRegexPack` sub-struct to decouple from the monolithic `ApiRegexPack`. Enables a clean path to tree-sitter integration when the grammar ABI is compatible.
- [ ] **C/C++ support**: Add public API extraction for `.h`/`.hpp` headers — function declarations, `extern "C"` blocks, struct/class definitions. High value for plugin/SDK scenarios with a C ABI layer.
- [ ] **IDE/LSP integration**: VS Code extension or language server that shows inline warnings when a public symbol is modified without a corresponding version bump.
- [ ] **GitHub Actions output annotations for `generate`**: Emit `::notice::` lines for auto-derived version increments so PR checks show exactly which axis bumped and why.
- [ ] **Plugin/custom extractor protocol**: Allow users to plug in their own language extractor via a subprocess protocol (stdin/stdout JSON) or WASM module, for niche DSLs or generated code.
- [ ] **Larger fixture corpora**: Add integration test fixtures from real open-source projects for each supported language to stress-test edge cases in export following and API boundary detection.
- [ ] **Replace test `panic!` placeholders**: `src/commands/generator.rs` test code uses `panic!("unexpected strategy")` as a fallback assertion. Replace with proper `assert!` or `assert_eq!` calls.

---

## Backlog (Post-1.x)
- [ ] **Zig / Nim / Odin support**: Emerging systems languages used in plugin/embedding contexts.
- [ ] **Ruby metaprogramming coverage**: `define_method`, `method_missing`, eval-based definitions. Intentionally deferred past 1.x.
- [ ] **Python dynamic `__all__` assembly**: Purely dynamic or metaprogramming-heavy export patterns. Deferred by design.
- [ ] **Lua/Luau complex module export tables**: Multiple return statements, conditional exports. Falls back to heuristics in 1.x.
- [ ] **Namespace-like Python layouts**: Deeper nested package hierarchies with namespace packages.
- [ ] **Changelog / release notes generation**: Given that MVSengine already knows exactly what changed (features, protocols, API drift), generate structured changelogs or release note drafts from manifest history.
