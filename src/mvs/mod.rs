// SPDX-License-Identifier: AGPL-3.0-only
// `hashing`, `manifest`, `project_detect`, `range`, `reader`, `schemes`, and
// `version_sources` now live in the standalone `mvs-core` crate (embeddable,
// no network/process dependencies); `crawler` lives in `mvs-crawler`
// (tree-sitter-heavy). Re-exported here under their historical paths so the
// rest of this crate's `crate::mvs::manifest::...`-style paths are
// unaffected by the split.
pub use mvs_core::{hashing, manifest, project_detect, range, reader, schemes, version_sources};
pub use mvs_crawler as crawler;

pub mod migrate;
pub mod suggest;
pub mod vcs;
