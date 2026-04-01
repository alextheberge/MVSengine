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
mvs-manager generate --root . --manifest mvs.json --context cli --exclude-path src/generated
mvs-manager generate --root . --manifest mvs.json --context cli --format json
mvs-manager lint --root . --manifest mvs.json
mvs-manager lint --root . --manifest mvs.json --format json
mvs-manager lint --root . --manifest mvs.json --available-model-capabilities tool_calling,json_schema,reasoning-v1
mvs-manager validate --host-manifest host.json --extension-manifest extension.json
mvs-manager validate --host-manifest host.json --extension-manifest extension.json --format json
mvs-manager validate --host-manifest host.json --extension-manifest extension.json --host-model-capabilities tool_calling,reasoning-v1
```

`mvs.json` persists version-change rationale in `history`, enabling compatibility reports to explain protocol breaks (for example, auth-flow changes tied to a specific `PROT`).

## Semantic Evidence Snapshots

`mvs-manager` now stores the semantic surfaces it reasons about, not just their hashes. The `evidence` block in `mvs.json` includes:

- `feature_inventory`: sorted unique `@mvs-feature(...)` tag names
- `protocol_inventory`: sorted unique `@mvs-protocol(...)` tag names
- `public_api_inventory`: sorted `file + canonical signature` entries for detected public API surfaces

This gives you two things:

- deterministic hashing for version-axis decisions
- machine-readable diffs explaining exactly what changed between manifest generations
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

The crawler now tokenizes source before matching decorators, uses AST extraction for Rust, and uses parser-backed public API adapters for every other supported language: TypeScript/JavaScript, Go, Python, Java, Kotlin, C#, PHP, Ruby, Swift, and Luau.

Internally, parser-backed extraction now dispatches through dedicated language adapters under `src/mvs/crawler/adapters/`, with shared language metadata in `src/mvs/crawler/language.rs`. That keeps new language work localized instead of expanding one monolithic extractor path.

- `@mvs-feature(...)` and `@mvs-protocol(...)` are counted only when they appear in real comments
- block comments such as `/* ... */` are supported for decorator extraction
- string literals and embedded fixture blobs are ignored during decorator extraction
- TypeScript/JavaScript public API extraction handles multiline exports, named export clauses, re-exports, and default exports without depending on line-based regex matching
- Go public API extraction tracks exported `func` declarations and exported methods from syntax trees
- Python public API extraction tracks non-underscore `def` declarations, including decorated class methods, without promoting nested local helpers into the API inventory
- Java and C# public API extraction tracks public types and public methods while stripping leading annotations or attributes out of the stored signature
- Kotlin public API extraction tracks public or default-visible `class`, `interface`, `object`, and `fun` declarations, preserving modifiers such as `data` and `suspend` while skipping `private`, `protected`, and `internal`
- PHP public API extraction tracks top-level functions and constants, classes, interfaces, traits, enums, public properties, public or interface constants, and public or interface methods while treating `#` comments as decorators and ignoring attribute syntax in stored signatures
- Ruby public API extraction tracks `class`, `module`, public `def`, singleton methods, and `class << self` method bodies while ignoring heredoc fixture content and methods hidden behind `private` or `protected`
- Swift public API extraction tracks `public` and `open` types, functions, properties, and inherited protocol requirements, and the scanner masks multiline Swift string literals so embedded examples do not pollute evidence
- Luau public API extraction tracks global `function` declarations, `export type` definitions, and returned module-table exports such as `Api.connect = function(...) end`, `function Api:refresh(...)`, and named fields from returned tables

This matters for repositories that keep code examples, fixture payloads, or prompt templates alongside real source. Those examples no longer pollute `mvs.json.evidence`.

## API Boundary Policy

You can persist scan policy in `mvs.json` so public API evidence reflects the contract you actually support instead of every reachable `pub` item.

The `scan_policy` block supports:

- `public_api_roots`: relative file or directory roots that define the public API boundary
- `public_api_includes`: wildcard patterns for declarations that should count as public API
- `public_api_excludes`: wildcard patterns for declarations that should not count as public API
- `exclude_paths`: relative file or directory prefixes to skip entirely during tag and API scanning

Example:

```json
{
  "scan_policy": {
    "public_api_roots": [
      "src/cli.rs"
    ],
    "public_api_excludes": [
      "rust:const EXIT_*",
      "rust:struct *Args",
      "rust:enum OutputFormat"
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
mvs-manager generate --root . --manifest mvs.json --context cli --public-api-include 'src/cli.rs|rust:fn *'
```

Practical guidance:

- CLI-first projects: point `public_api_roots` at the CLI facade such as `src/cli.rs`
- SDKs and libraries: point `public_api_roots` at exported facades such as `src/lib.rs`, `src/index.ts`, or `src/public/`
- Generated or vendor-like code under the root: add it to `exclude_paths`
- Use `public_api_excludes` when a facade file still contains public constants, argument structs, or helper exports that are not real compatibility surface
- Use `public_api_includes` when you want to pin the contract to a small explicit subset of declarations

This policy only scopes public API evidence. Feature and protocol tags are still gathered across the scanned codebase unless a path is explicitly excluded.

Pattern rules:

- `*` matches zero or more characters
- A plain pattern such as `rust:struct *Args` matches only the normalized signature
- A selector pattern such as `src/cli.rs|rust:fn *` matches both the relative file path and the signature
- If both include and exclude rules match the same declaration, the exclude rule wins
- The easiest way to author patterns is to copy a signature from `mvs.json.evidence.public_api_inventory` or `lint --format json`
- Legacy Rust function patterns with duplicated `fn` still match during migration, but regenerated manifests rewrite them to the canonical form

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

## License

This repository is licensed under the GNU Affero General Public License v3.0 only (`AGPL-3.0-only`).

- Full license text: [LICENSE](LICENSE)
- If you modify and run this software for users over a network, AGPL requires offering corresponding source to those users.
