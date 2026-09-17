// SPDX-License-Identifier: AGPL-3.0-only
//! `mvs-core`: the pure domain logic behind MVS (`ARCH.FEAT.PROT.FIX-CONT`)
//! multidimensional versioning — the manifest schema, legacy-version-scheme
//! translation, dependency-range resolution, host/extension compatibility
//! checks, and version-file read/write adapters.
//!
//! Deliberately dependency-light and free of network access or process
//! spawning, so it can be embedded directly (WASM, Node via napi, Python via
//! pyo3) without pulling in `mvs-manager`'s CLI, self-update, or
//! crawler/tree-sitter dependencies. The crawler lives in the separate
//! `mvs-crawler` crate, and git/process-shelling lives in `mvs-manager`
//! itself, since neither belongs in an embeddable core.

pub mod hashing;
pub mod manifest;
pub mod project_detect;
pub mod range;
pub mod reader;
pub mod schemes;
pub mod version_sources;
