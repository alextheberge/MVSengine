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

The parser-backed path is organized as per-language adapters, so expanding or tightening one language does not require editing the full crawler.

For class-like languages, stored member signatures are owner-qualified so collisions between similarly named methods or properties stay visible in `public_api_inventory`. Java, C#, and Kotlin also include declared package or namespace context in both type and member signatures.

- TypeScript/JavaScript: multiline exports, named export clauses, re-exports, and default exports are parser-backed
- Go: exported `func` declarations, exported methods, exported named types, exported struct fields, exported embedded struct fields, exported interface methods, embedded interface type elements, exported constants, and exported package `var` declarations are parser-backed
- Python: public `class` declarations, non-underscore `def` declarations, public `type` aliases, and module-level or class-level constants such as `API_VERSION`, `__all__`, or `Worker.STATUS` are parser-backed without promoting nested local helpers or private class bodies; parseable `__all__` becomes the top-level export boundary, and class methods are stored as `python:def Worker.run_job(...)`
- Java: public types, public fields, interface constants, and public or interface methods are parser-backed; stored signatures drop leading annotations and preserve package plus nesting context as `java:type public class demo.AuthApi`, `java:field public String demo.AuthApi.status`, `java:const public static final String demo.AuthApi.Contract.STATE`, and `java:method public String demo.AuthApi.login(...)`
- C#: public types, public fields, public constants, public properties, and public or interface methods are parser-backed; stored signatures drop leading attributes and preserve namespace plus nesting context as `csharp:type public class Demo.AuthApi`, `csharp:field public static readonly string Demo.AuthApi.Version`, `csharp:const public string Demo.AuthApi.STATUS_READY`, `csharp:property public string Demo.AuthApi.DisplayName { get }`, and `csharp:method public static string Demo.AuthApi.Login(...)`
- Kotlin: public or default-visible `class`, `interface`, `object`, `fun`, `val`, `var`, and top-level `const val` declarations are parser-backed, while `private`, `protected`, and `internal` declarations are skipped; stored signatures preserve package plus nesting context as `kotlin:public class demo.auth.AuthApi`, `kotlin:const val demo.auth.API_VERSION: String`, `kotlin:fun demo.auth.AuthApi.login(...)`, and `kotlin:val demo.auth.AuthApi.token: String`
- PHP: top-level functions and constants, classes, interfaces, traits, enums, public properties, public or interface constants, and public or interface methods are parser-backed; `#` comments count for decorators, attributes are ignored in stored signatures, and class/interface members are owner-qualified as `AuthApi.run(...)`, `AuthApi.$token`, and `AuthApi::STATUS_READY`
- Ruby: `class`, `module`, public `def`, singleton methods, `class << self` method bodies, public `attr_reader`/`attr_writer`/`attr_accessor` macros, and namespace constants are parser-backed; `private_constant` removes hidden constants, `#` comments count for decorators, heredocs plus non-public methods are ignored, and member signatures use Ruby owner forms such as `Demo::AuthApi#login(...)`
- Lua: global `function` declarations and returned module-table exports are parser-backed, `--` plus long-bracket comments are recognized during decorator scans, and `return Api`-style module roots become the explicit runtime export boundary
- Swift: `public` and `open` types, functions, properties, and inherited protocol requirements are parser-backed, multiline Swift string literals are masked during decorator scans, and type/protocol members are owner-qualified as `swift:public func AuthApi.login(...)` and `swift:public var SessionContract.token: ...`
- Luau: global `function` declarations, `export type` definitions, and returned module-table exports are parser-backed, `--` plus long-bracket comments are recognized during decorator scans, and `return Api`-style module roots become the explicit runtime export boundary while `export type` stays explicit API

If you already have Java, C#, or Kotlin entries in `public_api_inventory`, regenerate once so stored signatures pick up the new package or namespace prefixes.

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
