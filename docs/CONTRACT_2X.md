# 2.x Public Contract

This document defines the public compatibility contract for `mvs-manager` `2.x` releases (schema `https://mvs.dev/schema/v2`).

It supersedes the three-axis identity shape from [CONTRACT_1X.md](CONTRACT_1X.md). Host/extension compatibility rules, evidence inventories, scan policy, and command JSON remain additive where noted below.

## Identity

Canonical form:

```text
ARCH.FEAT.PROT.FIX-CONT
```

Stable `identity` fields:

- `mvs`
- `arch`
- `feat`
- `prot`
- `fix`
- `cont`

| Axis | Role | Compatibility impact |
|------|------|----------------------|
| `arch` | Data / system-generation break | Host/extension pairs typically share ARCH |
| `feat` | Feature surface (`@mvs-feature`) | None for `validate` |
| `prot` | Integration / public API / AI contract | Host and extension protocol ranges |
| `fix` | Bug fix / minor release | None for `validate` |
| `cont` | Deployment context label | Context hierarchy checks |

### SemVer projection

Package managers and git tags use **`ARCH.FEAT.FIX`** (three SemVer components). `prot` remains in `mvs.json` for compatibility matrices only.

Example: `1.10.4.4-cli` → Cargo/npm/tag `1.10.4`.

### Migration from v1

On load, `mvs-manager` migrates `https://mvs.dev/schema/v1` manifests:

1. Detect three-part `identity.mvs` (`ARCH.FEAT.PROT-CONT`)
2. Set `fix = prot` so SemVer `arch.feat.fix` preserves the legacy third digit
3. Rewrite `identity.mvs` to four-part form and set `$schema` to v2
4. Backfill history entries the same way

### Axis increment rules (`generate`)

- `--arch-break`: increment `arch`; reset `feat`/`prot`/`fix` to 0; then apply other increments
- Feature drift: increment `feat`; reset `fix` to 0
- Protocol / public API / AI schema drift: increment `prot`; reset `fix` to 0; then `fix += 1`
- `--fix` or `--auto-fix`: increment `fix` only when no ARCH/FEAT/PROT change

## Stable additions vs 1.x

### `generate --format json` identity

- `fix_increment` (in addition to `arch_increment`, `feat_increment`, `prot_increment`)

### `report --format json` identity comparison

- `fix_delta`

### `history[]`

- `fix`

### `release` (optional, additive)

- `release.version_files[]`: `{ path, kind, projection? }`, declaring which on-disk files `mvs-manager sync` keeps in lock-step with the SemVer projection (or the full MVS identity string, with `projection: "full"`). Omitted entirely when empty, so it never appears in manifests that predate it.
- The `sync` command and its `--format json` output are new in `2.x` and not yet covered by golden contract fixtures; treat its JSON shape as stabilizing rather than frozen until fixtures land.
- `convert-version` is a new, stateless command (no manifest is read or written) that parses a legacy version string and maps it onto MVS axes for preview purposes; see `docs/USAGE.md`. Its `--format json` output is likewise not yet pinned by golden fixtures.
- `migrate` (`detect`/`plan`/`backfill`/`apply`/`rollback`) is a new command family for adopting MVS in a project that currently uses another versioning scheme. `migrate apply` writes `mvs.json` and syncs `release.version_files` the same way `sync` does; `migrate rollback` reverses it from a snapshot at `.mvs/migration-snapshot.json`. None of its `--format json` output shapes are pinned by golden fixtures yet.
- `suggest-decorators` is a new command that proposes (and, with `--write`, inserts) `@mvs-feature`/`@mvs-protocol` comments for undecorated public API surface. It reads an existing manifest's `scan_policy` when present but never writes `mvs.json` itself — only source files, when `--write` is passed. Its `--format json` output isn't pinned by golden fixtures yet either.

Golden fixtures under `tests/fixtures/contracts/` are the review gate for this contract.
