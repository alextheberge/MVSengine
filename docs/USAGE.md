# Usage Guide

## 1) Generate or update `mvs.json`

```bash
mvs-manager generate --root . --manifest mvs.json --context cli
```

Machine-readable output:

```bash
mvs-manager generate --root . --manifest mvs.json --context cli --format json
```

The stable `1.x` manifest and command-output contract is defined in [docs/CONTRACT_1X.md](CONTRACT_1X.md).

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

`validate --format json` now includes structured `checks` entries with stable `axis`, `status`, and `code` fields so automation can distinguish protocol, context, capability, runtime-profile, and AI-contract failures without scraping human text.

Context hierarchies are supported. Example: `edge` extensions can run on `edge.mobile` hosts when ranges and capabilities pass.

AI runtime capability override for validation:

```bash
mvs-manager validate \
  --host-manifest host.json \
  --extension-manifest extension.json \
  --host-model-capabilities tool_calling,reasoning-v1
```

## 4) Compare two manifests directly

```bash
mvs-manager report --base-manifest old-mvs.json --target-manifest new-mvs.json
```

Machine-readable output:

```bash
mvs-manager report --base-manifest old-mvs.json --target-manifest new-mvs.json --format json
```

`report --format json` is the manifest-to-manifest diff command for bots and release automation. It reports:

- `change_count`
- `changed_sections`
- `comparison.identity`
- `comparison.compatibility`
- `comparison.capabilities`
- `comparison.ai_contract`
- `comparison.environment`
- `comparison.scan_policy`
- `comparison.evidence`

This is different from `lint`: `lint` compares code against the current manifest, while `report` compares one manifest against another manifest without crawling source.

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

When scan policy shapes the public boundary, `lint --format json` also emits `boundary_debug` so you can see which declarations were included or excluded and which rule or follow-mode decision caused that result.

Decorator extraction is comment-aware:

- real comments count
- block comments count
- string literals and embedded source examples do not count

Public API extraction is syntax-aware across all supported languages:

The parser-backed path is organized as per-language adapters, so expanding or tightening one language does not require editing the full crawler.

For class-like languages, stored member signatures are owner-qualified so collisions between similarly named methods or properties stay visible in `public_api_inventory`. Java, C#, and Kotlin also include declared package or namespace context in both type and member signatures.

- TypeScript/JavaScript: multiline exports, named export clauses, re-exports, and default exports are parser-backed; `scan_policy.ts_export_following` or `--ts-export-following relative-only` can follow same-workspace relative barrel re-exports, and `workspace-only` also follows same-workspace `package.json` export maps and `imports` maps, including wildcard subpaths, multi-condition entries, and package-local monorepo self-references, plus root `tsconfig.json` or `jsconfig.json` `baseUrl` and `paths`
- Go: exported `func` declarations, exported methods, exported named types, exported struct fields, exported embedded struct fields, exported interface methods, embedded interface type elements, exported constants, and exported package `var` declarations are parser-backed, and `scan_policy.go_export_following` or `--go-export-following package-only` can expand a rooted `.go` file to same-package sibling source files while skipping `_test.go` files
- Rust: AST-normalized `pub fn`, `pub struct`, `pub enum`, `pub trait`, `pub type`, `pub const`, `pub static`, and `pub` impl methods are parser-backed, and `scan_policy.rust_export_following` or `--rust-export-following public-modules` can expand a rooted Rust facade such as `src/lib.rs` across same-crate `pub mod` graphs, including nested inline public modules, while leaving private-module files out; direct and chained same-crate `pub use` facades, including grouped and glob reexports that stay inside the crate, are also resolved onto their public alias names, including associated inherent methods, and `scan_policy.rust_workspace_members` or `--rust-workspace-member` can explicitly allow selected sibling crates when a facade intentionally reexports workspace members, including chained facade crates and crates with nonstandard `[lib].path` roots
- Python: public `class` declarations, non-underscore `def` declarations, public `type` aliases, and module-level or class-level constants such as `API_VERSION`, `__all__`, or `Worker.STATUS` are parser-backed without promoting nested local helpers or private class bodies; parseable `__all__` becomes the top-level export boundary, including common alias, unpacking, and `+=` composition patterns built from parseable literals, explicit import re-exports are stored in canonical forms such as `python:from auth.core import login as authorize`, same-workspace `from ... import *` or imported `__all__` aliases resolve when the upstream module export graph is static and parseable, and `scan_policy.python_export_following` plus `scan_policy.python_module_roots` or `--python-module-root` can pin how cross-module facade resolution behaves
- Java: public types, public fields, interface constants, and public or interface methods are parser-backed; stored signatures drop leading annotations and preserve package plus nesting context as `java:type public class demo.AuthApi`, `java:field public String demo.AuthApi.status`, `java:const public static final String demo.AuthApi.Contract.STATE`, and `java:method public String demo.AuthApi.login(...)`
- C#: public types, public fields, public constants, public properties, and public or interface methods are parser-backed; stored signatures drop leading attributes and preserve namespace plus nesting context as `csharp:type public class Demo.AuthApi`, `csharp:field public static readonly string Demo.AuthApi.Version`, `csharp:const public string Demo.AuthApi.STATUS_READY`, `csharp:property public string Demo.AuthApi.DisplayName { get }`, and `csharp:method public static string Demo.AuthApi.Login(...)`
- Kotlin: public or default-visible `class`, `interface`, `object`, `fun`, `val`, `var`, and top-level `const val` declarations are parser-backed, while `private`, `protected`, and `internal` declarations are skipped; stored signatures preserve package plus nesting context as `kotlin:public class demo.auth.AuthApi`, `kotlin:const val demo.auth.API_VERSION: String`, `kotlin:fun demo.auth.AuthApi.login(...)`, and `kotlin:val demo.auth.AuthApi.token: String`
- PHP: top-level functions and constants, classes, interfaces, traits, enums, public properties, public or interface constants, and public or interface methods are parser-backed; `#` comments count for decorators, attributes are ignored in stored signatures, and class/interface members are owner-qualified as `AuthApi.run(...)`, `AuthApi.$token`, and `AuthApi::STATUS_READY`
- Ruby: `class`, `module`, public `def`, singleton methods, `class << self` method bodies, public `attr_reader`/`attr_writer`/`attr_accessor` macros, and namespace constants are parser-backed; `private_constant` hides constants until `public_constant` re-exposes them, `module_function` plus `extend self` surface module singleton exports, `private_class_method` hides singleton methods, `#` comments count for decorators, heredocs plus non-public methods are ignored, member signatures use Ruby owner forms such as `Demo::AuthApi#login(...)`, and `scan_policy.ruby_export_following` or `--ruby-export-following` can disable macro-driven export shaping
- Lua: global `function` declarations and returned module-table exports are parser-backed, `--` plus long-bracket comments are recognized during decorator scans, `return Api`-style module roots become the explicit runtime export boundary, and `scan_policy.lua_export_following` or `--lua-export-following` can disable returned-root following or require an explicit returned root before runtime exports are inferred
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
- `ts_export_following`: TypeScript/JavaScript barrel-following mode: `off`, `relative_only`, or `workspace_only`
- `go_export_following`: Go package expansion mode: `off` or `package_only`
- `rust_export_following`: Rust module-following mode: `off` or `public_modules`
- `rust_workspace_members`: relative crate directories or `Cargo.toml` paths that Rust facade following may resolve across when a facade intentionally reexports selected workspace members
- `ruby_export_following`: Ruby export-shaping mode: `heuristic` or `off`
- `lua_export_following`: Lua/Luau runtime export mode: `heuristic`, `returned_root_only`, or `off`
- `python_export_following`: Python cross-module export resolution mode: `heuristic`, `roots_only`, or `off`
- `public_api_includes`: wildcard rules for declarations that count as public API
- `public_api_excludes`: wildcard rules for declarations that should be ignored
- `python_module_roots`: relative directory roots used to resolve same-workspace Python module names for `__all__`, explicit re-exports, and wildcard imports
- `exclude_paths`: relative file or directory prefixes skipped by both tag and API scans

This is especially useful when:

- a CLI project exposes one facade file but keeps many internal `pub` helpers
- that facade file still contains public constants or argument structs that are not real consumer contract
- a library has an explicit `public/` or `index.ts` export layer
- a TypeScript or JavaScript repo uses barrel files and wants `index.ts` to contribute the followed concrete contract instead of raw re-export statements
- that TypeScript or JavaScript facade also depends on same-workspace `package.json` export maps, `imports` maps, package-local monorepo self-references, or `tsconfig` / `jsconfig` path aliases
- that `package.json` export or import maps use wildcard subpaths or multiple conditions and the repo wants workspace source targets to win over dist fallbacks
- a Go repo treats one `.go` file as the visible entrypoint but wants the whole same-package surface to count without importing `_test.go` helpers
- a Rust repo treats `src/lib.rs` or a workspace member crate facade as the contract root and wants same-crate `pub mod` files to count without scanning private-module files
- a Rust workspace wants one facade crate to reexport only selected sibling crates without resolving the whole workspace automatically
- a Ruby repo wants file-local declarations without `module_function` or `extend self` promotion
- a Lua or Luau repo wants explicit returned module roots before runtime exports count, or wants returned-root following disabled
- a Python repo keeps importable modules under a nonstandard root such as `app/` or `services/`
- a Python repo wants strict facade following under declared roots only, or wants cross-file export following disabled entirely
- generated code sits under the normal source root

Flags passed to `generate` persist into `mvs.json.scan_policy`, so later `lint` runs use the same boundary automatically.

If you are debugging why a declaration is missing from or leaking into `public_api_inventory`, prefer `lint --format json`: `boundary_debug` reports direct root matches, root misses, include/exclude selector matches, and file-level follow-mode decisions such as `package_only` or `public_modules`.

For `workspace_only`, package export and import targets are tried in this order when multiple conditions exist: `types`, `import`, `module`, `browser`, `node`, `default`, `require`, then any remaining custom conditions in key order. Root `tsconfig.json` is preferred over `jsconfig.json` when both exist.

Example:

```bash
mvs-manager generate --root . --manifest mvs.json --context server --public-api-root src/index.ts --ts-export-following relative-only
mvs-manager generate --root . --manifest mvs.json --context server --public-api-root src/index.ts --ts-export-following workspace-only
mvs-manager generate --root . --manifest mvs.json --context server --public-api-root src/api.go --go-export-following package-only
mvs-manager generate --root . --manifest mvs.json --context server --public-api-root src/lib.rs --rust-export-following public-modules
mvs-manager generate --root . --manifest mvs.json --context server --public-api-root app/src/lib.rs --rust-export-following public-modules --rust-workspace-member shared
mvs-manager generate --root . --manifest mvs.json --context server --ruby-export-following off
mvs-manager generate --root . --manifest mvs.json --context server --lua-export-following returned-root-only
mvs-manager generate --root . --manifest mvs.json --context server --python-module-root app
mvs-manager generate --root . --manifest mvs.json --context server --python-export-following roots-only --python-module-root app
```

Pattern matching rules:

- `*` matches zero or more characters
- `rust:struct *Args` matches signatures only
- `src/cli.rs|rust:fn *` matches a relative file path and a signature together
- exclude rules win over include rules when both match the same declaration
- legacy Rust function patterns like `rust:fn fn *` still match during migration, but `generate` rewrites inventories to the canonical form

See also: [docs/CONTRACT_1X.md](CONTRACT_1X.md) for the frozen `1.x` manifest and command-output contract, and [docs/TODO_1.0.md](TODO_1.0.md) for the remaining release-readiness work.

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
