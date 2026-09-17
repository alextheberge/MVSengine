# Migrating an npm/Node project to MVS

Applies to a plain `package.json`, npm/pnpm/yarn workspaces, and Node-based
monorepos.

## What `mvs-manager` detects

- `package.json#version` (root, and every workspace member listed under
  `workspaces` / `pnpm-workspace.yaml`) as the version source.
- The existing scheme is almost always `semver` (npm enforces SemVer at
  publish time), so the default mapping is `MAJOR->ARCH, MINOR->FEAT,
  PATCH->FIX`, with `PROT` starting at `0` until inferred.
- Public API surface: parser-backed TS/JS export scanning, including named
  exports, re-exports, multiline exports, default exports, and — when
  `scan_policy.workspace_only` is set — package `exports`/`imports` maps
  (including wildcard subpaths and monorepo self-references) and
  `tsconfig.json`/`jsconfig.json` `baseUrl`/`paths`.

## Steps

```bash
mvs-manager migrate detect --root .
mvs-manager migrate plan --root .
# review the printed legacy-tag -> MVS-tag table, then, optionally:
mvs-manager migrate backfill --tags v1.0.0..HEAD
mvs-manager migrate apply --root .
```

`apply` adds `release.version_files` pointing at every `package.json` it
found and writes `mvs.json` at the repo root. From then on:

```bash
mvs-manager sync           # write the ARCH.FEAT.FIX projection into every package.json
mvs-manager sync --check   # CI gate: fail if any package.json disagrees with mvs.json
```

## Registry continuity

npm refuses to let you publish a version lower than what's already on the
registry for that package. `migrate plan` checks this invariant before you
apply: the proposed `ARCH.FEAT.FIX` projection for the current HEAD must be
`>=` the last published npm version. If your repo's history includes
0.x releases with breaking changes hidden in minor bumps (common under
ZeroVer-style npm conventions), `migrate backfill` will surface those in its
SemVer honesty report — expect `ARCH` to jump higher than your current
`MAJOR` in that case, which is the correct outcome, not a bug.

## CI

Add to your existing workflow (see the composite [GitHub Action](../../.github/actions/setup-mvs)
or [../INSTALL_AND_CI.md](../INSTALL_AND_CI.md)):

```yaml
- uses: alextheberge/mvs-action@v1
  with:
    mode: lint
- run: mvs-manager sync --check
```

## Monorepo note

`migrate detect`/`plan` walk every `package.json` under `--root`, so a single
`mvs.json` at the workspace root with `release.version_files` listing each
member's `package.json` covers a fixed/locked-versioning workspace (all
packages bump together, e.g. Lerna's default mode) out of the box — `sync`
then writes the same projection to every listed file. If your workspace
versions packages independently instead, run `migrate detect`/`plan`/`apply`
separately with `--root <package-dir>` for each package that needs its own
`mvs.json`.
