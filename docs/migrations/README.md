# Migration guides

Ecosystem-specific walkthroughs for adopting MVS on an existing project. All
of them follow the same shape:

```bash
mvs-manager migrate detect      # what version scheme(s) and sources exist today
mvs-manager migrate plan        # proposed mvs.json + legacy-tag -> MVS-tag table
mvs-manager migrate backfill --tags <range>   # optional: SemVer honesty report from git history
mvs-manager migrate apply       # write mvs.json + sync version files (snapshot saved for rollback)
mvs-manager lint                # confirm the new manifest passes
mvs-manager sync --check        # confirm every version file agrees with the manifest
mvs-manager migrate rollback    # undo apply, restoring the pre-migration files
```

Every write step (`apply`, `sync`) is safe to `--dry-run` first, and `apply`
always records a snapshot so `rollback` is a clean revert — see
[../USAGE.md](../USAGE.md) for full flag reference. See [../SPEC.md](../SPEC.md)
for what the `ARCH.FEAT.PROT.FIX` axes mean and how they project onto SemVer.

- [npm-and-node.md](npm-and-node.md)
- [cargo-and-rust.md](cargo-and-rust.md)
- [python.md](python.md)

Any ecosystem not yet listed still works through the generic flow above —
`migrate detect` reports the scheme it found (`semver`, `zerover`, `pep440`,
`maven-gradle`, `dotnet4`, `go-modules`, `calver`, `integer`, `debian-rpm`, or
`custom`) and the default axis mapping for it; override with
`--map major=arch,minor=feat,patch=fix` or `--scheme-regex` for anything
project-specific.
