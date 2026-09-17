# Migrating a Python project to MVS

Applies to PEP 621 (`pyproject.toml#project.version`), Poetry
(`pyproject.toml#tool.poetry.version`), Hatch, `setup.cfg`, and plain
`__version__` module attributes.

## What `mvs-manager` detects

- Version source: `pyproject.toml` (PEP 621 or Poetry section),
  `setup.cfg`, or a `__version__ = "..."` constant, per
  `crates/mvs-core/src/version_sources.rs`.
- Scheme is usually `pep440`. PEP 440's epoch, `.postN`, and `.devN`
  segments are handled explicitly: an epoch folds into an `ARCH` offset,
  `.postN` maps to `FIX`, `.devN` becomes prerelease metadata rather than a
  compatibility axis. Plain `X.Y.Z` PEP 440 versions behave like `semver`.
- Public API surface: parser-backed scanning of public `class`/`def`
  declarations, module- and class-level constants, and `__all__` (including
  common alias/unpacking/`+=` composition patterns) as the export boundary
  when it's statically parseable. `scan_policy.python_export_following` and
  `scan_policy.python_module_roots` control how aggressively re-export
  facades (`from x import *`, imported `__all__` aliases) are followed
  across modules.

## Steps

```bash
mvs-manager migrate detect --root .
mvs-manager migrate plan --root .
mvs-manager migrate backfill --tags v1.0.0..HEAD   # optional
mvs-manager migrate apply --root .
mvs-manager sync --check
```

## ZeroVer projects

A large fraction of Python packages stay on `0.y.z` indefinitely
(NumPy-style ZeroVer) and treat minor bumps as breaking. `migrate detect`
reports this as the `zerover` scheme, and `migrate backfill` will flag minor
bumps in your git tag history that changed the public API — those become
candidates for a higher `ARCH` than your current `MAJOR: 0` suggests. Review
the backfill report before `apply`; don't let a long ZeroVer history silently
become `ARCH: 0` forever if the actual break history says otherwise.

## `PROT` for library vs. application code

Most Python libraries only ship one integration surface (the package's own
public API), so `PROT` tracks the same public-API inventory. If your project
also serves an API (FastAPI/Flask routes, an RPC layer, a plugin ABI), keep
that boundary separate with `@mvs-protocol` decorators so `PROT` reflects
wire/plugin compatibility independently of the Python-level `ARCH`/`FEAT`
surface — see [../USAGE.md](../USAGE.md) for decorator placement, or run
`mvs-manager suggest-decorators` to get a starting point from your existing
export boundaries.
