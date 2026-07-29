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

Golden fixtures under `tests/fixtures/contracts/` are the review gate for this contract.
