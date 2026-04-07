# MVS Engine (`mvs-manager`)

Cross-platform CLI implementing multidimensional versioning: `[ARCH].[FEAT].[PROT]-[CONT]`.

## Why Post-SemVer for Apps
- Separate data breaks from integration breaks: `ARCH` and `PROT` move independently.
- Stop silent plugin/SDK regressions: `lint` fails when public API or AI contract drift is unaccounted.
- Keep compatibility explicit: protocol ranges, legacy shims, capabilities, and context live in `mvs.json`.

## Quick Start (4 Steps)

### 1) Install `mvs-manager`

### macOS/Linux
```bash
curl -fsSL https://raw.githubusercontent.com/alextheberge/MVSengine/master/scripts/install.sh | bash
```

### Windows PowerShell
```powershell
irm https://raw.githubusercontent.com/alextheberge/MVSengine/master/scripts/install.ps1 | iex
```

Once installed, `mvs-manager` can update itself with:

```bash
mvs-manager self-update
```

Interactive runs of the CLI also notify on `stderr` when a newer stable release is available.

### 2) Initialize your project manifest
```bash
mvs-manager generate --root . --manifest mvs.json --context cli
```

### 3) Add decorators in your code
Decorator syntax can be function-style or `:` style:

```ts
// @mvs-feature("offline_storage")
// @mvs-protocol("auth-api-v1")
export function login(user: string): string {
  return user;
}
```

```rs
/// @mvs-feature("runtime_bridge")
/// @mvs-protocol("host_extension_handshake")
pub fn handshake(version: u32) -> bool {
    version > 0
}
```

Only actual source comments are scanned. Examples embedded in string literals, test fixtures, or documentation blobs inside source files do not count as decorators.

### 4) Run the initialize/verify/reconcile loop
```bash
# verify current code matches mvs.json
mvs-manager lint --root . --manifest mvs.json

# after code/decorator/API changes, reconcile and persist rationale/history
mvs-manager generate --root . --manifest mvs.json --context cli
```

## Enforce MVS on Commit
```bash
make install-hooks
```

This installs a repo-managed `pre-commit` hook that runs `make lint-manifest`.

## GitHub Release Setup (Required for Installer)

1. Enable Actions with write access:
   - Repository Settings -> Actions -> Workflow permissions -> **Read and write permissions**
2. Keep dogfood versions aligned:
   ```bash
   make dogfood-sync-version
   make ci
   ```
3. Push version changes to `main` or `master`:
   - `Auto Tag Version` workflow creates canonical tag automatically.
   - If `mvs.json` is `0.2.3-cli`, canonical tag is `v0.2.3`.
4. Wait for `Auto Tag Version` to dispatch `Release` and publish assets:
   - `mvs-manager-<version>-<target>.<tar.gz|zip>`
   - `checksums.txt`
   - If needed, run `Auto Tag Version` manually from GitHub Actions (`Run workflow`) to force tag/release dispatch.

The installer (`.../scripts/install.sh`) depends on these release assets.

One-command flow from repo root:
```bash
make release-github
```

Release-candidate flow:
```bash
make release-rc RELEASE_TAG_SUFFIX=rc1 RELEASE_ALLOW_NON_DEFAULT=true
```

`latest` installers intentionally ignore prerelease tags. To install an RC explicitly, set `MVS_VERSION=vX.Y.Z-rcN`.

## Local Build
```bash
make ci
make build-release
```

## Core Commands
```bash
mvs-manager generate --root . --manifest mvs.json --context cli
mvs-manager generate --root . --manifest mvs.json --context edge.mobile --backwards-compatible 3
mvs-manager generate --root . --manifest mvs.json --context cli --public-api-root src/cli.rs
mvs-manager generate --root . --manifest mvs.json --context cli --public-api-root src/api.go --go-export-following package-only
mvs-manager generate --root . --manifest mvs.json --context cli --public-api-root src/lib.rs --rust-export-following public-modules
mvs-manager generate --root . --manifest mvs.json --context cli --exclude-path src/generated
mvs-manager generate --root . --manifest mvs.json --context cli --format json
mvs-manager lint --root . --manifest mvs.json
mvs-manager lint --root . --manifest mvs.json --format json
mvs-manager lint --root . --manifest mvs.json --available-model-capabilities tool_calling,json_schema,reasoning-v1
mvs-manager validate --host-manifest host.json --extension-manifest extension.json
mvs-manager validate --host-manifest host.json --extension-manifest extension.json --format json
mvs-manager validate --host-manifest host.json --extension-manifest extension.json --host-model-capabilities tool_calling,reasoning-v1
mvs-manager report --base-manifest old-mvs.json --target-manifest new-mvs.json
mvs-manager report --base-manifest old-mvs.json --target-manifest new-mvs.json --format json
mvs-manager self-update --check
mvs-manager self-update
```

`mvs.json` persists version-change rationale in `history`, enabling compatibility reports to explain protocol breaks (for example, auth-flow changes tied to a specific `PROT`).

The stable `1.x` manifest and command-output contract is documented in [docs/CONTRACT_1X.md](docs/CONTRACT_1X.md). Checked-in golden fixtures under `tests/fixtures/contracts/` back that contract so output and canonical evidence changes stay deliberate.

## Semantic Evidence Snapshots

`mvs-manager` now stores the semantic surfaces it reasons about, not just their hashes. The `evidence` block in `mvs.json` includes:

- `feature_inventory`: sorted unique `@mvs-feature(...)` tag names
- `protocol_inventory`: sorted unique `@mvs-protocol(...)` tag names
- `public_api_inventory`: sorted `file + canonical signature` entries for detected public API surfaces

This gives you two things:

- deterministic hashing for version-axis decisions
- machine-readable diffs explaining exactly what changed between manifest generations
- machine-readable boundary debugging in `generate --format json` and `lint --format json` when scan policy shapes the public surface
- scanner precision that ignores decorator-like examples inside source string literals

Typical `evidence` shape:

```json
{
  "evidence": {
    "feature_hash": "…",
    "protocol_hash": "…",
    "public_api_hash": "…",
    "feature_inventory": [
      "manifest_generation",
      "manifest_linting"
    ],
    "protocol_inventory": [
      "cli_generate_command",
      "cli_lint_command"
    ],
    "public_api_inventory": [
      {
        "file": "src/cli.rs",
        "signature": "rust:enum Command"
      }
    ]
  }
}
```

If an older manifest predates these inventory snapshots, `lint` will fail until you regenerate once. That is intentional: it brings the manifest up to the current evidence model.

Rust API entries are now AST-normalized into stable, readable forms such as:

- `rust:fn run() -> i32`
- `rust:impl-fn HostAdapter::connect(&self, target: &str) -> bool`
- `rust:fn async load<'a, T>(value: &'a T) -> &'a T where T: Clone`

This reduces formatting noise in `public_api_inventory` and makes policy patterns easier to author.

## Scanner Precision

The crawler now tokenizes source before matching decorators, uses AST extraction for Rust, and uses parser-backed public API adapters for every other supported language: TypeScript/JavaScript, Go, Python, Java, Kotlin, C#, PHP, Ruby, Swift, Lua, Luau, and Dart (Dart uses line-regex extraction today because the published Tree-sitter grammar targets a newer ABI than the `tree-sitter` 0.24 stack pinned here; see `docs/USAGE.md`). Shopify-style Liquid (`.liquid`) is scanned for decorators inside HTML comments, `{% comment %}…{% endcomment %}`, and `{% # … %}` tags; templates do not emit `public_api` entries.

Internally, parser-backed extraction now dispatches through dedicated language adapters under `src/mvs/crawler/adapters/`, with shared language metadata in `src/mvs/crawler/language.rs`. That keeps new language work localized instead of expanding one monolithic extractor path.

Member signatures inside class-like scopes are now owner-qualified, and Java/C#/Kotlin signatures also include declared package or namespace context so inventories stay stable across files and modules. Typical forms include `python:def Worker.run_job(...)`, `ruby:def Demo::AuthApi#login(...)`, `java:type public class demo.AuthApi`, `java:field public String demo.AuthApi.status`, `csharp:property public string Demo.AuthApi.DisplayName { get }`, `kotlin:const val demo.auth.API_VERSION: String`, `kotlin:val demo.auth.AuthApi.token: String`, `php:public readonly string AuthApi.$token`, and `swift:public func AuthApi.login(...)`.

- `@mvs-feature(...)` and `@mvs-protocol(...)` are counted only when they appear in real comments
- block comments such as `/* ... */` are supported for decorator extraction
- string literals and embedded fixture blobs are ignored during decorator extraction
- TypeScript/JavaScript public API extraction handles multiline exports, named export clauses, re-exports, and default exports without depending on line-based regex matching; `scan_policy.ts_export_following` or `--ts-export-following` can follow same-workspace relative barrel re-exports, and `workspace_only` mode also follows same-workspace `package.json` export maps and `imports` maps, including wildcard subpaths, multi-condition entries, and package-local monorepo self-references, plus root `tsconfig.json` or `jsconfig.json` `baseUrl` and `paths` aliases so facade files contribute the concrete signatures they export instead of raw `export ... from` statements
- Go public API extraction tracks exported `func` declarations, exported methods, exported named types, exported struct fields, exported embedded struct fields, exported interface methods, embedded interface type elements, exported constants, and exported package `var` declarations from syntax trees; `scan_policy.go_export_following` or `--go-export-following` can expand a rooted `.go` facade file to same-package sibling source files while skipping `_test.go` files
- Rust public API extraction still uses AST-normalized signatures, and `scan_policy.rust_export_following` or `--rust-export-following` can expand a rooted Rust facade such as `src/lib.rs` across same-crate `pub mod` graphs, including nested inline public modules, without pulling in private-module files or `tests`/`examples`/`benches`; direct and chained same-crate `pub use` facades, including grouped and glob reexports that stay inside the crate, are also resolved onto the public alias names, including associated inherent methods, and `scan_policy.rust_workspace_members` or `--rust-workspace-member` can explicitly allow selected sibling crates when a facade intentionally reexports workspace members, including chained facade crates and crates with nonstandard `[lib].path` roots
- Python public API extraction tracks public `class` and non-underscore `def` declarations, public `type` aliases, module-level constants such as `API_VERSION` or `__all__`, and public class-level constants such as `Worker.STATUS`, without promoting nested local helpers or private class bodies into the API inventory; when a parseable `__all__` is present it becomes the top-level export boundary, including common alias, unpacking, and `+=` composition patterns built from parseable literals, and explicit import re-exports are stored in canonical forms such as `python:from auth.core import login as authorize`; for same-workspace Python modules, imported `__all__` aliases and `from ... import *` re-exports also resolve when the source module export graph is static and parseable, and `scan_policy.python_export_following` plus `scan_policy.python_module_roots` or `--python-module-root` can pin how aggressively module resolution follows nonstandard repository roots
- Java public API extraction tracks public types, public fields, interface constants, and public or interface methods while stripping leading annotations out of the stored signature; declared package plus nesting context is preserved in canonical forms such as `java:type public class demo.AuthApi`, `java:field public String demo.AuthApi.status`, `java:const public static final String demo.AuthApi.Contract.STATE`, and `java:method public String demo.AuthApi.login(...)`
- C# public API extraction tracks public types, public fields, public constants, public properties, and public or interface methods while stripping leading attributes out of the stored signature; declared namespace plus nesting context is preserved in canonical forms such as `csharp:type public class Demo.AuthApi`, `csharp:field public static readonly string Demo.AuthApi.Version`, `csharp:const public string Demo.AuthApi.STATUS_READY`, `csharp:property public string Demo.AuthApi.DisplayName { get }`, and `csharp:method public static string Demo.AuthApi.Login(...)`
- Kotlin public API extraction tracks public or default-visible `class`, `interface`, `object`, `fun`, `val`, `var`, and top-level `const val` declarations, preserving modifiers such as `data` and `suspend` while skipping `private`, `protected`, and `internal`; declared package plus nesting context is preserved in canonical forms such as `kotlin:public class demo.auth.AuthApi`, `kotlin:const val demo.auth.API_VERSION: String`, `kotlin:fun demo.auth.AuthApi.login(...)`, and `kotlin:val demo.auth.AuthApi.token: String`
- PHP public API extraction tracks top-level functions and constants, classes, interfaces, traits, enums, public properties, public or interface constants, and public or interface methods while treating `#` comments as decorators and ignoring attribute syntax in stored signatures; class and interface members are owner-qualified as `AuthApi.run(...)`, `AuthApi.$token`, and `AuthApi::STATUS_READY`
- Ruby public API extraction tracks `class`, `module`, public `def`, singleton methods, `class << self` method bodies, public `attr_reader`/`attr_writer`/`attr_accessor` macros, and constant assignments within public namespaces while ignoring heredoc fixture content; `private_constant` hides constants until `public_constant` re-exposes them, `module_function` and `extend self` promote module exports into singleton signatures, `private_class_method` removes hidden singleton methods, and member signatures use Ruby owner forms such as `Demo::AuthApi#login(...)` and `Demo::AuthApi.connect(...)`; `scan_policy.ruby_export_following` or `--ruby-export-following` can keep that macro-driven export shaping enabled or reduce Ruby scanning to file-local declarations only
- Lua public API extraction tracks global `function` declarations and returned module-table exports such as `Api.connect = function(...) end`, `function Api:refresh(...)`, and named fields from returned tables, while `--` and long-bracket comments remain decorator-aware; when a file returns a module root such as `return Api`, that returned root becomes the export boundary and unrelated globals stop counting; `scan_policy.lua_export_following` or `--lua-export-following` can disable returned-root following or require explicit returned roots before runtime exports are inferred
- Swift public API extraction tracks `public` and `open` types, functions, properties, and inherited protocol requirements, and the scanner masks multiline Swift string literals so embedded examples do not pollute evidence; type and protocol members are owner-qualified as `swift:public func AuthApi.login(...)` and `swift:public var SessionContract.token: ...`
- Luau public API extraction tracks global `function` declarations, `export type` definitions, and returned module-table exports such as `Api.connect = function(...) end`, `function Api:refresh(...)`, and named fields from returned tables; when a file returns a module root such as `return Api`, that returned root becomes the runtime export boundary while `export type` declarations remain explicit API
- Dart public API extraction uses comment-aware scanning plus line-regex heuristics for classes, mixins, enums, extensions, typedefs, fields, getters, setters, and callables; a `library` directive qualifies signatures with a dotted prefix, and names starting with `_` are skipped as library-private. Typical stored forms include `dart:type demo.class AuthApi`, `dart:field demo.static const String VERSION =`, and `dart:function demo.String login(String username)`

If you already track Java, C#, or Kotlin API evidence, expect one regeneration after this change because canonical signatures now include declared package or namespace prefixes.

This matters for repositories that keep code examples, fixture payloads, or prompt templates alongside real source. Those examples no longer pollute `mvs.json.evidence`.

## API Boundary Policy

You can persist scan policy in `mvs.json` so public API evidence reflects the contract you actually support instead of every reachable `pub` item.

The `scan_policy` block supports:

- `public_api_roots`: relative file or directory roots that define the public API boundary
- `ts_export_following`: TypeScript/JavaScript barrel-following mode: `off` (default), `relative_only`, or `workspace_only`
- `go_export_following`: Go package expansion mode: `off` (default) or `package_only`
- `rust_export_following`: Rust module-following mode: `off` (default) or `public_modules`
- `rust_workspace_members`: relative crate directories or `Cargo.toml` paths that Rust facade following may resolve across when a facade intentionally reexports selected workspace members
- `ruby_export_following`: Ruby export-shaping mode: `heuristic` (default) or `off`
- `lua_export_following`: Lua/Luau runtime export mode: `heuristic` (default), `returned_root_only`, or `off`
- `python_export_following`: Python cross-module export resolution mode: `heuristic` (default), `roots_only`, or `off`
- `public_api_includes`: wildcard patterns for declarations that should count as public API
- `public_api_excludes`: wildcard patterns for declarations that should not count as public API
- `python_module_roots`: relative directory roots used to resolve same-workspace Python module names before following `__all__`, explicit re-exports, or `from ... import *` facades
- `exclude_paths`: relative file or directory prefixes to skip entirely during tag and API scanning

Example:

```json
{
  "scan_policy": {
    "public_api_roots": [
      "src/cli.rs"
    ],
    "ts_export_following": "workspace_only",
    "go_export_following": "package_only",
    "rust_export_following": "public_modules",
    "rust_workspace_members": [
      "crates/shared"
    ],
    "ruby_export_following": "off",
    "lua_export_following": "returned_root_only",
    "python_export_following": "roots_only",
    "public_api_excludes": [
      "rust:const EXIT_*",
      "rust:struct *Args",
      "rust:enum OutputFormat"
    ],
    "python_module_roots": [
      "app"
    ],
    "exclude_paths": [
      "src/generated"
    ]
  }
}
```

You can set these during generation:

```bash
mvs-manager generate --root . --manifest mvs.json --context cli --public-api-root src/cli.rs
mvs-manager generate --root . --manifest mvs.json --context cli --exclude-path src/generated
mvs-manager generate --root . --manifest mvs.json --context cli --public-api-exclude 'rust:const EXIT_*'
mvs-manager generate --root . --manifest mvs.json --context cli --public-api-root app/src/lib.rs --rust-export-following public-modules --rust-workspace-member shared
mvs-manager generate --root . --manifest mvs.json --context cli --public-api-include 'src/cli.rs|rust:fn *'
mvs-manager generate --root . --manifest mvs.json --context cli --public-api-root src/index.ts --ts-export-following relative-only
mvs-manager generate --root . --manifest mvs.json --context cli --public-api-root src/index.ts --ts-export-following workspace-only
mvs-manager generate --root . --manifest mvs.json --context cli --public-api-root src/api.go --go-export-following package-only
mvs-manager generate --root . --manifest mvs.json --context cli --public-api-root src/lib.rs --rust-export-following public-modules
mvs-manager generate --root . --manifest mvs.json --context cli --ruby-export-following off
mvs-manager generate --root . --manifest mvs.json --context cli --lua-export-following returned-root-only
mvs-manager generate --root . --manifest mvs.json --context cli --python-export-following roots-only --python-module-root app
mvs-manager generate --root . --manifest mvs.json --context cli --python-module-root app
```

Practical guidance:

- CLI-first projects: point `public_api_roots` at the CLI facade such as `src/cli.rs`
- SDKs and libraries: point `public_api_roots` at exported facades such as `src/lib.rs`, `src/index.ts`, or `src/public/`
- Generated or vendor-like code under the root: add it to `exclude_paths`
- Use `public_api_excludes` when a facade file still contains public constants, argument structs, or helper exports that are not real compatibility surface
- Use `public_api_includes` when you want to pin the contract to a small explicit subset of declarations
- TypeScript or JavaScript repos with only relative barrel facades: set `ts_export_following` to `relative_only`
- TypeScript or JavaScript repos with `package.json` export maps, `imports` maps, package-local monorepo self-references, or `tsconfig`/`jsconfig` path aliases inside the same workspace: set `ts_export_following` to `workspace_only`
- When `workspace_only` follows `package.json` `exports` or `imports` conditions, it prefers `types`, then `import`, `module`, `browser`, `node`, `default`, and `require`; any remaining custom conditions are tried after that in key order
- When both root config files exist, `workspace_only` reads `tsconfig.json` before `jsconfig.json`
- Go repos that root the contract on one `.go` file but ship a whole package surface: set `go_export_following` to `package_only` so same-package sibling files count without dragging `_test.go` helpers into `public_api_inventory`
- Rust repos that root the contract on `src/lib.rs` or another crate facade file: set `rust_export_following` to `public_modules` so same-crate `pub mod` files count with that facade while private-module files stay out
- Rust workspaces where one facade crate intentionally reexports selected sibling crates: add `rust_workspace_members` so only those member crates resolve across `pub use member_crate::...` paths instead of widening to the entire workspace
- Ruby repos that want file-local declarations only: set `ruby_export_following` to `off`
- Lua or Luau repos that require explicit runtime module returns: set `lua_export_following` to `returned_root_only`
- Lua or Luau repos that want file-local globals only: set `lua_export_following` to `off`
- Python repos with nonstandard package roots: set `python_module_roots`
- Python repos that want strict facade tracking: set `python_export_following` to `roots_only` and provide `python_module_roots`
- Python repos that want file-local behavior only: set `python_export_following` to `off`

This policy only scopes public API evidence. Feature and protocol tags are still gathered across the scanned codebase unless a path is explicitly excluded.

Pattern rules:

- `*` matches zero or more characters
- A plain pattern such as `rust:struct *Args` matches only the normalized signature
- A selector pattern such as `src/cli.rs|rust:fn *` matches both the relative file path and the signature
- If both include and exclude rules match the same declaration, the exclude rule wins
- The easiest way to author patterns is to copy a signature from `mvs.json.evidence.public_api_inventory` or `lint --format json`
- Legacy Rust function patterns with duplicated `fn` still match during migration, but regenerated manifests rewrite them to the canonical form

## Roadmap

- Usage details: [docs/USAGE.md](docs/USAGE.md)
- Release workflow: [docs/RELEASE.md](docs/RELEASE.md)
- `1.x` readiness roadmap: [docs/TODO_1.0.md](docs/TODO_1.0.md)

## Machine-Readable Output

All commands support `--format text|json` and default to `text`.

Examples:

```bash
mvs-manager generate --root . --manifest mvs.json --context cli --format json
mvs-manager lint --root . --manifest mvs.json --format json
mvs-manager validate --host-manifest host.json --extension-manifest extension.json --format json
```

JSON responses are designed for CI, bots, editor tooling, and release automation. They include:

- `command`
- `status`
- `exit_code`
- semantic diff details where relevant
- command-specific metadata such as identity changes, inventory counts, or compatibility reasons

When scan policy actively shapes the public API boundary, `generate --format json` and `lint --format json` also emit `boundary_debug` with included and excluded candidate declarations, excluded paths, and the matched root/include/exclude rule or follow-mode reason. Default ignored directories such as `tests`, `target`, and `node_modules` also show up there when they affect the crawl.

`validate --format json` also includes:

- `failure_count`
- `degraded_count`
- `checks`: structured compatibility checks with stable `axis`, `status`, and `code` values

`report --format json` also includes:

- `change_count`
- `changed_sections`
- `comparison`: structured manifest-to-manifest deltas across identity, compatibility, capabilities, AI contract, environment, scan policy, and evidence inventories

`report` intentionally stays manifest-only in `1.x`. It does not recrawl source or emit `boundary_debug`.

Example `lint --format json` failure shape:

```json
{
  "command": "lint",
  "status": "failed",
  "exit_code": 20,
  "failure_count": 1,
  "failures": [
    "Public API signature drift detected. Added: src/api.ts|ts/js:function rotateToken(token:string): string Build must fail until PROT is incremented and manifest is regenerated."
  ]
}
```

Example `validate --format json` incompatibility shape:

```json
{
  "command": "validate",
  "status": "incompatible",
  "exit_code": 30,
  "compatible": false,
  "degraded": false,
  "failure_count": 1,
  "degraded_count": 0,
  "checks": [
    {
      "axis": "protocol",
      "status": "fail",
      "code": "protocol_range_mismatch",
      "message": "Protocol range mismatch: extension requires host 1-1, host exposes 2-2 and is at PROT 2.",
      "details": {
        "extension_protocol": 1,
        "host_protocol": 2,
        "required_host_range": {
          "min_prot": 1,
          "max_prot": 1
        },
        "host_extension_range": {
          "min_prot": 2,
          "max_prot": 2
        }
      }
    }
  ],
  "reasons": [
    "Protocol range mismatch: extension requires host 1-1, host exposes 2-2 and is at PROT 2."
  ]
}
```

Example `report --format json` shape:

```json
{
  "command": "report",
  "status": "changed",
  "exit_code": 0,
  "change_count": 21,
  "changed_sections": [
    "identity",
    "compatibility",
    "evidence"
  ]
}
```

## Stable Exit Codes

`mvs-manager` now uses command-stable nonzero exit codes so automation can distinguish drift from execution failures.

- `0`: success
- `10`: `generate` execution failure
- `20`: `lint` detected manifest/code drift or policy failure
- `21`: `lint` execution failure
- `30`: `validate` found incompatibility
- `40`: manifest read/parse/write/validation failure
- `70`: output rendering failure

This means CI can treat `20` as “manifest must be regenerated” and `30` as “host/extension contract is incompatible” without scraping human text.

## Release + Verification

Release assets include:
- per-platform archives
- `checksums.txt`
- optional `checksums.txt.asc` GPG signature

Verify checksum:
```bash
scripts/verify-release.sh dist/vX.Y.Z/mvs-manager-X.Y.Z-<target>.tar.gz dist/vX.Y.Z/checksums.txt
```

See [docs/USAGE.md](docs/USAGE.md) and [docs/RELEASE.md](docs/RELEASE.md) for complete workflows.
For the frozen `1.x` manifest and command-output contract, see [docs/CONTRACT_1X.md](docs/CONTRACT_1X.md).

## License

This repository is licensed under the GNU Affero General Public License v3.0 only (`AGPL-3.0-only`).

- Full license text: [LICENSE](LICENSE)
- If you modify and run this software for users over a network, AGPL requires offering corresponding source to those users.
