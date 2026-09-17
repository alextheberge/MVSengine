# MVSengine (`mvs-manager`)

Rust CLI implementing multidimensional versioning (`ARCH.FEAT.PROT.FIX-CONT`).
Cargo workspace: `mvs-core` (manifest/hashing/schemes/version_sources, no
network/process deps), `mvs-crawler` (tree-sitter scanners), `mvs-manager`
(root package, the CLI), `mvs-wasm` (WASM bindings over `mvs-core`).

Before changing behavior: `make ci` (fmt, clippy, full workspace test suite,
dogfood lint cycle against this repo's own `mvs.json`). Golden fixtures in
`tests/fixtures/contracts/` gate any change to manifest shape or
machine-readable command output — see [docs/CONTRACT_2X.md](docs/CONTRACT_2X.md).

## Deferred work (tracked, not forgotten)

As of 2026-09-16 (Phase 6, commit `4dbd993`), the following were scoped out
and are still open — pick any of these up when asked, don't silently assume
they're done because related work landed:

- **Registry publishing.** Nothing has been published to crates.io, npm,
  PyPI, a Homebrew tap, `winget-pkgs`, a Docker registry, or a Nix channel.
  `packaging/homebrew/`, `packaging/scoop/`, `packaging/winget/` are
  templates with placeholder `0000...` checksums — fill them in from a real
  release's `checksums.txt` before submitting anywhere. See
  [packaging/README.md](packaging/README.md).
- **Docker/Nix are unbuilt.** [`Dockerfile`](Dockerfile) and
  [`flake.nix`](flake.nix) were written against standard patterns but never
  build-tested (no Docker daemon / no `nix` binary were available when
  written). Run `docker build -t mvs-manager .` and `nix build` /
  `nix flake check` before trusting either.
- **`napi-rs`/`pyo3` bindings.** Only the WASM binding
  ([`crates/mvs-wasm`](crates/mvs-wasm)) exists. `mvs-core`'s functions are
  already plain `Result<T, String>`, so a Node/Python binding is mostly the
  thin-FFI-boundary work `crates/mvs-wasm/src/lib.rs`'s
  `convert_version_inner` split already demonstrates.
- **6 more language crawlers**: Elixir, Scala, C/C++, Zig, Haskell, OCaml —
  not started. Also recheck the Dart tree-sitter ABI (noted as blocked in
  the README).
- **Non-code protocol surfaces**: OpenAPI, GraphQL SDL, `.proto`, JSON
  Schema, Avro aren't crawled as public-API/protocol inventory yet.
- **`mvs.dev` hosting.** `mvs.json`'s `$schema` points at
  `https://mvs.dev/schema/v2`, which isn't live. The published
  [interactive playground](https://claude.ai/artifact/XWg1XtbGqvtNXSwQSFHeaF)
  is a standalone artifact, not hosted on `mvs.dev` — needs a domain owner
  to register/host it.

Full rationale for each item, plus the cross-repo adoption feedback loop
(other repos that have adopted MVS and known gaps found while integrating
it), lives in this assistant's persistent memory
(`mvsengine-cross-repo-integration` note) — ask to have it summarized if
picking this work back up in a fresh session that doesn't have it loaded.

See also: [README.md](README.md)'s Roadmap section, and
[docs/DISTRIBUTION.md](docs/DISTRIBUTION.md) for the full channel-by-channel
status.
