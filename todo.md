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
