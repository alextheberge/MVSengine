# The MVS Versioning Specification

This document defines Multidimensional Versioning (MVS) as a versioning scheme,
independent of `mvs-manager` or any other implementation. It exists so other
tools — linters, dependency resolvers, registries, editor extensions — can
implement or consume MVS identities without depending on this repository.
`mvs-manager` is the reference implementation; its manifest format and CLI
contract are documented separately in [CONTRACT_2X.md](CONTRACT_2X.md).

## 1. Motivation

SemVer (`MAJOR.MINOR.PATCH`) collapses two independent questions into one
number: *"will this break the code that calls me?"* and *"will this break the
data/state this software has already persisted, or the wire protocol it
speaks to other processes?"* A single `MAJOR` bump conflates a renamed
function with a changed on-disk format with a changed HTTP contract. Callers
cannot tell, from the version number alone, which kind of break occurred.

MVS splits compatibility into four axes plus one identity tag, so a version
number states precisely what changed.

## 2. Identity format

```
ARCH.FEAT.PROT.FIX-CONT
```

| Component | Meaning | Increments when |
|---|---|---|
| `ARCH` | Architecture / data compatibility | Persisted state, storage format, or migration compatibility changes in a way old data cannot be read, or a breaking architectural change occurs. |
| `FEAT` | Feature surface | A capability is added, removed, or its observable behavior changes, without breaking `ARCH` or `PROT`. |
| `PROT` | Protocol / integration compatibility | The public API, wire protocol, plugin/extension contract, or any machine-to-machine boundary changes incompatibly for existing callers. |
| `FIX` | Patch / fix | A backward-compatible bug fix or internal change with no `ARCH`/`FEAT`/`PROT` impact. |
| `CONT` | Context | A free-form runtime/target label (e.g. `cli`, `web`, `server`, `edge`, `desktop`) identifying which build or deployment target this identity describes. Not a compatibility axis — informational only. |

Each of `ARCH`, `FEAT`, `PROT`, `FIX` is a non-negative integer. `CONT` is an
opaque string matching `[a-z0-9][a-z0-9.-]*` (dot-segments allowed for
nested contexts, e.g. `edge.mobile`).

### 2.1 Ordering

Two identities are compared lexicographically over `(ARCH, FEAT, PROT, FIX)`
as a 4-tuple of integers. `CONT` does not participate in ordering — identities
with different `CONT` values are only meaningfully comparable when they
describe the same runtime target. Implementations MUST NOT compare across
different `CONT` values and treat the result as a compatibility judgment.

### 2.2 Independence rule

`ARCH` and `PROT` move independently: a change that breaks stored data but
not the API bumps `ARCH` only; a change that breaks the API but not stored
data bumps `PROT` only. A change breaking both bumps both. This is the core
property SemVer cannot express with a single `MAJOR`.

## 3. Compatibility ranges

A dependency on an MVS-versioned artifact is expressed as independent
constraints per axis, e.g. `arch=2, prot>=1 <3, feat>=4`. An implementation
MAY choose to only constrain the axes it cares about (a pure data consumer
constrains `ARCH` only; a pure API client constrains `PROT` only).

`mvs-manager range --for <ecosystem>` (see [USAGE.md](USAGE.md)) is one
projection of an MVS range into a native SemVer-based range for ecosystems
that don't understand MVS natively — see §4.

## 4. The SemVer projection

Because most registries (npm, crates.io, PyPI, Maven, etc.) only understand
`MAJOR.MINOR.PATCH`, an MVS identity projects onto SemVer as:

```
MAJOR = ARCH
MINOR = FEAT
PATCH = FIX
```

`PROT` is dropped from the published SemVer tag and instead recorded in the
project's MVS manifest (e.g. `mvs.json`'s `identity.prot` field) for tools
that read it directly. This means a `PROT`-only change (e.g. a webhook
payload shape changing but no code/API/data format changing) still needs a
compatibility signal in the SemVer world — implementations SHOULD bump
`FEAT` (→ `MINOR`) or document the `PROT` change prominently in release
notes when it must be visible to consumers who only see the SemVer tag.

This projection is one-directional and lossy: `ARCH.FEAT.FIX` can always be
derived from `ARCH.FEAT.PROT.FIX`, but `PROT` cannot be recovered from a bare
SemVer tag alone. Consumers who need `PROT` fidelity must read the MVS
manifest, not just the registry version.

## 5. Determining which axis moved

This specification does not mandate a single algorithm for detecting axis
changes — that is a tooling concern (see `mvs-manager lint` and
`report --format json` in [USAGE.md](USAGE.md) for one approach based on
diffing a hashed public-API inventory and a persisted-schema inventory
between two manifest snapshots). It does require that:

- A conforming tool MUST be able to explain, in machine-readable form, *why*
  it believes an axis changed (which symbols/files/schema entries differed).
- A conforming tool MUST NOT silently downgrade a detected `PROT` or `ARCH`
  break to a smaller axis without an explicit, auditable override.

## 6. Non-goals

- MVS does not replace SemVer for consumers who only need "is this
  compatible" — the projection in §4 exists precisely so those consumers
  never need to know MVS exists.
- MVS does not define a package manager, registry, or dependency resolver.
  It defines an identity format and a compatibility-range shape that any of
  those can adopt.
- `CONT` is not a compatibility axis and MUST NOT be used in compatibility
  comparisons (§2.1).

## 7. Reference implementation

`mvs-manager` (this repository) implements: manifest storage and hashing
(`mvs-core`), public-API/protocol-surface extraction for 14+ languages
(`mvs-crawler`), the CLI and lint/migrate/report workflow (`mvs-manager`),
and WASM/CLI embeddings of the version-scheme translation logic. See
[CONTRACT_2X.md](CONTRACT_2X.md) for exactly what its manifest and
machine-readable command output guarantee, and [migrations/](migrations/)
for moving an existing SemVer/CalVer/etc. project onto MVS identities.
