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

Use `--fix` for bug-fix / minor releases when evidence did not drift:

```bash
mvs-manager generate --root . --manifest mvs.json --context cli --fix
```

`--auto-fix` bumps FIX only when no ARCH/FEAT/PROT change (handy for release remediation). Package SemVer is `ARCH.FEAT.FIX`; see [CONTRACT_2X.md](CONTRACT_2X.md).

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

## 2b) Run periodic maintenance

Interactive maintenance loop:

```bash
mvs-manager watch --root . --manifest mvs.json --remediate --interval-secs 30
```

Scheduler-friendly single pass:

```bash
mvs-manager watch --root . --manifest mvs.json --once --remediate
```

`watch` reuses the normal lint flow, so `--explain`, `--ai-schema`, and `--available-model-capabilities` still apply. By default it fingerprints the workspace and skips unchanged cycles; add `--run-every-interval` if you want a strict cadence regardless of filesystem changes. Use `--strict-fingerprint` to exit with a non-zero code when fingerprinting fails instead of warning and running lint anyway.

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

### `validate-all` (host/extension matrix)

```bash
mvs-manager validate-all --dir ./packages --format json
```

`--format json` includes a `compatibility` object with `incompatible` and `degraded` short lists (for bots) in addition to the full `pairs` matrix.

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

This is different from `lint`: `lint` compares code against the current manifest, while `report` compares one manifest against another manifest without crawling source. `report` intentionally stays manifest-only in `1.x`, so scan-path and boundary-debug reasoning lives on the crawl-based commands instead.

## 5) Sync package version files

`sync` writes the SemVer projection (`identity.arch.identity.feat.identity.fix`, or the full MVS identity string with `"projection": "full"`) into every file declared in `mvs.json`'s `release.version_files`. When that list is empty, it auto-detects well-known files at the project root (`Cargo.toml`, `package.json`, `pyproject.toml`, `composer.json`, `pubspec.yaml`, `gradle.properties`, `build.gradle(.kts)`, `pom.xml`, `*.csproj`, `*.gemspec`, `*.rockspec`, `VERSION`) instead of requiring every project to declare it up front:

```bash
mvs-manager sync --root . --manifest mvs.json --check        # CI gate: nonzero on drift, writes nothing
mvs-manager sync --root . --manifest mvs.json --dry-run       # preview what would change
mvs-manager sync --root . --manifest mvs.json                 # write the projection into declared/detected files
mvs-manager sync --root . --manifest mvs.json --save-detected # persist auto-detected files into release.version_files
mvs-manager sync --root . --manifest mvs.json --suffix rc1    # append a prerelease suffix, e.g. 2.1.0-rc1
```

Only the version's byte span is rewritten; surrounding formatting, key order, and comments are left untouched. `mvs-manager sync --format json` reports a per-file `status` of `in_sync`, `updated`, `would_update`, `drift`, or `error`.

## 6) Convert a legacy version string

`convert-version` parses one version string under a known scheme and maps its components onto MVS axes, without touching any file. It's the tool for previewing a migration before committing to it:

```bash
mvs-manager convert-version --version 1.4.2
mvs-manager convert-version --version 1.4.2 --scheme semver --format json
mvs-manager convert-version --version 0.3.1                                   # auto-detected as zerover
mvs-manager convert-version --version 24.04                                   # auto-detected as calver
mvs-manager convert-version --version "R7-F12-P3" --scheme custom \
  --scheme-regex '(?P<arch>\d+)-F(?P<feat>\d+)-P(?P<prot>\d+)'
```

Supported `--scheme` values, auto-detected when omitted, with their default `component -> axis` mapping:

| Scheme | Shape | Components | Default mapping |
|---|---|---|---|
| `semver` | `MAJOR.MINOR.PATCH[-pre][+build]`, `MAJOR >= 1` | major, minor, patch | major→arch, minor→feat, patch→fix |
| `zerover` | Same shape, `MAJOR == 0` | major, minor, patch | same as SemVer; advisory notes minor bumps are often breaking |
| `pep440` | `[N!]N[.N[.N]][{a\|b\|c\|rc}N][.postN][.devN]` | major, minor, patch (+ epoch) | major→arch, minor→feat, patch→fix; epoch folds additively into arch |
| `maven-gradle` | `MAJOR.MINOR[.PATCH][-QUALIFIER]` | major, minor, patch | major→arch, minor→feat, patch→fix |
| `dotnet4` | `A.B.C.D` | major, minor, patch, revision | major→arch, minor→feat, patch→fix; `revision` unmapped by default (advisory) |
| `go-modules` | SemVer with an optional leading `v` | major, minor, patch | same as SemVer; advisory notes the `/vN` import-path requirement when arch >= 2 |
| `calver` | `YYYY.MM[.patch]` or `YY.MM[.patch]` | year, month, patch | **rebase** (default): arch=1, feat=0, patch→fix; pass `--map year=arch,month=feat,patch=fix` for date-passthrough |
| `integer` | A bare non-negative integer | build | arch=1, feat=0, build→fix |
| `debian-rpm` | `[EPOCH:]UPSTREAM[-REVISION]` | major, minor, patch (+ epoch, revision) | major→arch, minor→feat, patch→fix; epoch folds additively into arch; revision is dropped (metadata only) |
| `custom` | A `--scheme-regex` with named capture groups | whatever the regex names | group names equal to `arch`/`feat`/`prot`/`fix` map directly; others need `--map` |

`--map <spec>` (e.g. `--map major=arch,minor=feat,patch=fix`) replaces the default mapping entirely. `--prot <N>` overrides the resolved PROT axis afterward, since no legacy scheme encodes API/protocol compatibility on its own. This command only prints the result; it does not write `mvs.json`.

## 7) Migrate a project end to end

`migrate` has five subcommands, all under `mvs-manager migrate <subcommand>`:

```bash
mvs-manager migrate detect --root .
mvs-manager migrate plan --root . --context cli --format json
mvs-manager migrate backfill --root . --limit 30 --format json
mvs-manager migrate apply --root . --context cli
mvs-manager migrate rollback --root .
```

### `detect`

Read-only. Reports:
- every recognized version file (`release.version_files` auto-detection from `sync`) and whether they agree with each other
- git availability, whether `--root` is a repo, tag count, and the latest tag
- the scheme auto-detected from the latest tag (or the first version file if there are no tags)
- release tooling it recognizes by config-file presence: semantic-release, release-please, changesets, GitVersion, Nerdbank.GitVersioning, cargo-release, goreleaser, lerna — plus real file-list imports from `.bumpversion.cfg` and `tbump.toml`
- detected project languages (reusing the same detector `init` uses to build a scan policy)

### `plan`

Read-only; never writes `mvs.json`. Converts a source version into a proposed manifest:

- **Source selection**, in order: `--from-version`, else the latest git tag, else the first detected version file. Fails clearly if none is available.
- **Scheme/mapping**: same `--scheme`/`--scheme-regex`/`--map` as `convert-version` (see section 6). `--prot <N>` sets PROT explicitly.
- **Monotonic invariant**: the proposed SemVer projection (`arch.feat.fix`) must be `>=` the latest published tag's projection under the same scheme/mapping, so a migration can never make a registry regress. Violating this is a hard error unless `--allow-non-monotonic` is passed.
- **Scan policy**: built the same way `init` builds one (`--preset library|cli|plugin|plugin-host|sdk` supported).
- **`release.version_files`**: every auto-detected version file, plus any file `.bumpversion.cfg`/`tbump.toml` named that maps to a known `VersionFileKind` (see section 5). Files it can't confidently classify are listed separately as `unrecognized_imported_version_files` rather than guessed at.
- **Tag preview**: every git tag converted under the same scheme/mapping, so you can see the whole history projected into MVS before committing to it.

### `backfill`

Requires `git`. Checks out each of the most recent `--limit` tags (or an explicit `--tags a,b,c`) into a disposable `git worktree`, crawls it with the same crawler `generate`/`lint` use, and diffs consecutive snapshots via `Evidence::semantic_diff`. For each tag-to-tag transition it reports:

- `bump_level`: `major`/`minor`/`patch`/`none`/`unknown`, classified from the two tags' resolved MVS axes
- `features_added`/`features_removed`, `protocols_added`/`protocols_removed`, `public_api_added`/`public_api_removed`
- a `violation` when either:
  - the transition was **patch**-level and the protocol/public-API surface changed at all (patch releases should carry zero surface change), or
  - the transition was **minor**-level and the surface had **removals** (additions are fine in a minor under conventional SemVer; removals are a breaking change regardless of what the version number says)

The scan policy comes from the existing `mvs.json`'s `scan_policy` if one is present at `--manifest`, otherwise one is built the same way `init`/`plan` build one. That policy (in particular `public_api_roots`) is evaluated against every historical tag, which is a known simplification if the project's structure changed significantly over time.

### `apply`

Runs the same conversion as `plan` and persists it:

1. Refuses to run if `--manifest` already exists, unless `--force`.
2. Saves everything it's about to touch — the current `mvs.json` bytes (if any) and every `release.version_files` entry's current contents — to `.mvs/migration-snapshot.json`.
3. Writes the proposed `mvs.json`.
4. Calls the same `sync` write path for every declared version file.

If step 4 fails partway through, the manifest and snapshot are already on disk; run `migrate rollback` to undo everything cleanly.

### `rollback`

Reads `.mvs/migration-snapshot.json`, restores `mvs.json` (or deletes it, if it didn't exist before `apply`) and every backed-up version file to their exact prior contents, then deletes the snapshot. A second `rollback` with nothing left to restore fails clearly rather than silently doing nothing.

## 8) Bootstrap decorators

```bash
mvs-manager suggest-decorators --root .
mvs-manager suggest-decorators --root . --write
mvs-manager suggest-decorators --root . --write --format json
```

`suggest-decorators` crawls the source tree (using an existing manifest's `scan_policy` if `--manifest` resolves to one, otherwise building one the same way `init` does) and proposes:

- **Protocol suggestions**: one per file that has public API surface (from `public_api_inventory`) but no `@mvs-protocol` tag anywhere in that file yet.
- **Feature suggestions**: one per directory of such files with no `@mvs-feature` tag anywhere in it yet — a "capability cluster" — placed on the alphabetically-first file in that directory.

Both `@mvs-feature`/`@mvs-protocol` tags are scoped to the whole file they appear in (the crawler collects every comment in a file, not just ones attached to a specific declaration), so a suggestion only needs to land somewhere sensible in the file, not exactly above one symbol. Names are derived from the file/directory path, normalized to `snake_case`; a name is replaced with a matching Conventional Commits scope's own spelling (`feat(scope): ...` in `git log`) when one is found, so a team's existing naming convention wins over a guessed one. Name collisions get a numeric suffix (`auth`, `auth_2`, ...).

`--write` inserts each suggestion using the target file's own comment syntax, right after any leading header comment or shebang (so a license header stays first):

| Language(s) | Extensions | Comment used |
|---|---|---|
| Rust | `.rs` | `///` |
| TS/JS, Go, Java, Kotlin, C#, Swift, Dart | `.ts`, `.tsx`, `.js`, `.jsx`, `.go`, `.java`, `.kt`, `.cs`, `.swift`, `.dart` | `//` |
| Python, Ruby | `.py`, `.rb` | `#` |
| Lua, Luau | `.lua`, `.luau` | `--` |
| PHP | `.php` | `//`, inserted right after the `<?php` opening tag (a `.php` file is HTML/text outside it) |

Any other extension is reported as a suggestion but not written (`written: false` in JSON), since there's no comment syntax to insert safely. Because already-decorated files/directories are never suggested again, `--write` is idempotent — run it twice and the second run reports `no_suggestions`.

**Not yet built**: non-code protocol surfaces. Many teams' real protocol is a schema (OpenAPI, GraphQL SDL, `.proto`, JSON Schema, Avro), not a function signature, and none of those are crawled as public API inventory today.

## Makefile shortcuts

```bash
make install-hooks
make generate
make lint-manifest
make validate
make sync
make sync-check
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

When scan policy shapes the public boundary, `generate --format json` and `lint --format json` also emit `boundary_debug` so you can see which declarations were included or excluded, which directories or files were skipped, and which rule or follow-mode decision caused that result.

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
- Dart: `library` names qualify signatures with a dotted prefix; classes, mixins, enums, `extension` / `extension type`, `typedef` aliases (equals form), static const fields, getters, setters, and function-like lines ending with `{`, `=> ...;`, or `;` are captured with line- and regex-based heuristics (names starting with `_` are treated as library-private and skipped). The published `tree-sitter-dart` grammar targets a newer Tree-sitter ABI than the `tree-sitter` 0.24 line used here, so Dart intentionally uses this regex path until the engine upgrades. Typical entries look like `dart:type demo.class AuthApi`, `dart:field demo.static const String VERSION =`, and `dart:function demo.String login(String username)`
- Liquid (`.liquid`): HTML `<!-- … -->` comments, `{% comment %}…{% endcomment %}`, and inline `{% # … %}` tags are scanned for `@mvs-feature` / `@mvs-protocol` decorators; output tags (`{{ … }}`), other Liquid tags, and plain markup are masked so decorators embedded in strings or templates are ignored. Liquid files do not contribute `public_api` inventory.

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

If you are debugging why a declaration is missing from or leaking into `public_api_inventory`, prefer `generate --format json` or `lint --format json`: `boundary_debug` reports direct root matches, root misses, include/exclude selector matches, file-level follow-mode decisions such as `package_only` or `public_modules`, explicit `exclude_paths`, and default ignored directories such as `tests` or `target`.

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

## Doctor and environment

```bash
mvs-manager doctor
mvs-manager doctor --format json --root .
```

Prints the resolved binary path, optional `PATH` resolution, `MVS_REPO`-derived GitHub URLs, update-check-related environment, and presence of tools used by the installers (`curl`, `tar`, `sha256sum` / `shasum`, `bash`, `powershell`).

## Forks and `self-update`

- Set **`MVS_REPO`** (or **`MVS_UPDATE_REPO`**) to `owner/name` so `self-update`, `install.sh`, and `doctor` all target your fork’s releases and raw scripts on `raw.githubusercontent.com`.
- **`self-update`** refuses to run when the current executable looks like a **Cargo** build (`target/debug` / `target/release`), lives under **`.cargo`**, is in the **Nix store**, or when the install directory is not writable—unless **`MVS_ALLOW_UNSAFE_SELF_UPDATE=1`** is set. Use a normal install prefix (for example `$HOME/.local/bin`) or re-run `scripts/install.sh` with `MVS_INSTALL_DIR`.

## Prereleases and the GitHub API

Update checks use the **`releases/latest`** API endpoint, which returns the newest **non-prerelease** tag. Pre-release tags are not surfaced there; install a prerelease explicitly with `MVS_VERSION=v1.2.3-rc1` and `install.sh` / `install.ps1`.

## CI and pinned installs

See [docs/INSTALL_AND_CI.md](INSTALL_AND_CI.md) for GitHub Actions examples, pinning, and environment variables (`MVS_NO_UPDATE_CHECK`, tokens, and checksum-verified installs).

## Exit Codes

- `0`: success
- `10`: `generate` execution failure
- `20`: `lint` detected drift
- `21`: `lint` execution failure
- `30`: `validate` incompatibility
- `40`: manifest read/parse/write/validation failure
- `70`: output rendering failure
- `80`: `sync --check` found version files out of sync with the manifest projection
- `81`: `sync` failed to read or write a version file
- `82`: `convert-version` could not parse the input, resolve `--scheme-regex`, or apply `--map`
- `83`: a `migrate` subcommand failed (detect/plan/backfill/apply/rollback)
- `84`: `suggest-decorators` failed to crawl the source tree or write a decorator

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
