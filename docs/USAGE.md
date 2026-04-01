# Usage Guide

## 1) Generate or update `mvs.json`

```bash
mvs-manager generate --root . --manifest mvs.json --context cli
```

Machine-readable output:

```bash
mvs-manager generate --root . --manifest mvs.json --context cli --format json
```

Persist an explicit API boundary:

```bash
mvs-manager generate --root . --manifest mvs.json --context cli --public-api-root src/cli.rs
```

Keep only specific declarations from that boundary:

```bash
mvs-manager generate --root . --manifest mvs.json --context cli --public-api-root src/cli.rs --public-api-include 'src/cli.rs|rust:fn *'
```

Drop public-but-non-contract declarations from that boundary:

```bash
mvs-manager generate --root . --manifest mvs.json --context cli --public-api-root src/cli.rs --public-api-exclude 'rust:const EXIT_*'
```

Skip generated or vendor-like paths under the scan root:

```bash
mvs-manager generate --root . --manifest mvs.json --context cli --exclude-path src/generated
```

Use `--arch-break` for explicit data/schema breaks:

```bash
mvs-manager generate --root . --manifest mvs.json --context cli --arch-break --arch-reason "persistent schema migration"
```

Range strategy flags:

```bash
# lock host/extension protocol ranges to current PROT
mvs-manager generate --root . --manifest mvs.json --context cli --lock-step

# declare compatibility window and auto-generate shim declarations
mvs-manager generate --root . --manifest mvs.json --context cli --backwards-compatible 3
```

Every increment rationale is persisted to `mvs.json.history`.

## 2) Build-gate with linter

```bash
mvs-manager lint --root . --manifest mvs.json
```

Machine-readable output:

```bash
mvs-manager lint --root . --manifest mvs.json --format json
```

Optional AI schema drift checks:

```bash
mvs-manager lint --root . --manifest mvs.json --ai-schema ./tool_schema.json
```

AI liveness checks (runtime capability validation):

```bash
mvs-manager lint --root . --manifest mvs.json --available-model-capabilities tool_calling,json_schema,reasoning-v1
```

## 3) Validate host/extension compatibility

```bash
mvs-manager validate --host-manifest host.json --extension-manifest extension.json --allow-shims true
```

Machine-readable output:

```bash
mvs-manager validate --host-manifest host.json --extension-manifest extension.json --format json
```

Context hierarchies are supported. Example: `edge` extensions can run on `edge.mobile` hosts when ranges and capabilities pass.

AI runtime capability override for validation:

```bash
mvs-manager validate \
  --host-manifest host.json \
  --extension-manifest extension.json \
  --host-model-capabilities tool_calling,reasoning-v1
```

## Makefile shortcuts

```bash
make install-hooks
make generate
make lint-manifest
make validate
make ci
make build-release
```

`make install-hooks` sets `core.hooksPath` to `.githooks` and enables a pre-commit gate that runs `make lint-manifest`.

## Semantic Evidence

`generate` writes both hashes and semantic inventories into `mvs.json.evidence`:

- `feature_inventory`
- `protocol_inventory`
- `public_api_inventory`

`lint` checks these snapshots in addition to the hashes. If your manifest was created before inventories existed, regenerate once to bring it forward.

Decorator extraction is comment-aware:

- real comments count
- block comments count
- string literals and embedded source examples do not count

Public API extraction is syntax-aware across all supported languages:

- TypeScript/JavaScript: multiline exports, named export clauses, re-exports, and default exports are parser-backed
- Go: exported `func` declarations and exported methods are parser-backed
- Python: non-underscore `def` declarations, including decorated class methods, are parser-backed without promoting nested local helpers
- Java and C#: public type and method declarations are parser-backed, and stored signatures drop leading annotations or attributes
- Kotlin: public or default-visible `class`, `interface`, `object`, and `fun` declarations are parser-backed, while `private`, `protected`, and `internal` declarations are skipped
- PHP: top-level functions and constants, classes, interfaces, traits, enums, public properties, public or interface constants, and public or interface methods are parser-backed; `#` comments count for decorators, while attributes are ignored in stored signatures
- Swift: `public` and `open` types, functions, properties, and inherited protocol requirements are parser-backed, and multiline Swift string literals are masked during decorator scans
- Luau: global `function` declarations, `export type` definitions, and returned module-table exports are parser-backed, and `--` plus long-bracket comments are recognized during decorator scans

Rust API signatures are AST-normalized before they are persisted. Typical entries look like:

- `rust:fn run() -> i32`
- `rust:impl-fn HostAdapter::connect(&self, target: &str) -> bool`
- `rust:fn async load<'a, T>(value: &'a T) -> &'a T where T: Clone`

## Scan Policy

`mvs.json.scan_policy` lets you narrow API evidence to real contract boundaries:

- `public_api_roots`: relative file or directory prefixes that define the public API surface
- `public_api_includes`: wildcard rules for declarations that count as public API
- `public_api_excludes`: wildcard rules for declarations that should be ignored
- `exclude_paths`: relative file or directory prefixes skipped by both tag and API scans

This is especially useful when:

- a CLI project exposes one facade file but keeps many internal `pub` helpers
- that facade file still contains public constants or argument structs that are not real consumer contract
- a library has an explicit `public/` or `index.ts` export layer
- generated code sits under the normal source root

Flags passed to `generate` persist into `mvs.json.scan_policy`, so later `lint` runs use the same boundary automatically.

Pattern matching rules:

- `*` matches zero or more characters
- `rust:struct *Args` matches signatures only
- `src/cli.rs|rust:fn *` matches a relative file path and a signature together
- exclude rules win over include rules when both match the same declaration
- legacy Rust function patterns like `rust:fn fn *` still match during migration, but `generate` rewrites inventories to the canonical form

## Exit Codes

- `0`: success
- `10`: `generate` execution failure
- `20`: `lint` detected drift
- `21`: `lint` execution failure
- `30`: `validate` incompatibility
- `40`: manifest read/parse/write/validation failure
- `70`: output rendering failure

## Troubleshooting

### `Lint failed ... Public API signature drift detected`
- Run `mvs-manager generate` and commit updated `mvs.json`.
- If this is a true integration break, verify `PROT` increment rationale in output.

### `Checksum mismatch`
- Re-download archive + checksums.
- Confirm you are validating the matching release tag.
- If mismatch persists, treat artifact as untrusted.

### `Protocol range mismatch`
- Update host/extension ranges in `mvs.json`.
- Add a `legacy_shims` adapter only if degraded compatibility is intentionally supported.

### Installer cannot find release
- Confirm `MVS_REPO` points to `alextheberge/MVSengine`.
- Ensure tag exists and archive target matches your OS/CPU.
