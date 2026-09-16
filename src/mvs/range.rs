// SPDX-License-Identifier: AGPL-3.0-only
//! Converts a PROT compatibility range into native dependency-constraint
//! syntax for common package managers, so a downstream consumer gets value
//! from `mvs.json` without adopting MVS themselves.
//!
//! PROT isn't part of the registry-facing SemVer projection (`arch.feat.fix`),
//! so there's no arithmetic way to turn `[min_prot, max_prot]` into a version
//! range — it has to be looked up. This walks the manifest's `history`
//! (append-only, written by every `generate`) to find which published
//! `arch.feat.fix` versions actually had a PROT value inside the range, and
//! emits an inclusive lower bound and, when a later published version with a
//! different PROT is known, an exclusive upper bound. When no later version
//! is known, the range is left open above — compatibility going forward is
//! unknown, not assumed.

use anyhow::{bail, Result};

use crate::mvs::manifest::{Identity, Manifest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecosystem {
    Npm,
    Cargo,
    Pip,
    Maven,
}

impl Ecosystem {
    pub fn as_str(&self) -> &'static str {
        match self {
            Ecosystem::Npm => "npm",
            Ecosystem::Cargo => "cargo",
            Ecosystem::Pip => "pip",
            Ecosystem::Maven => "maven",
        }
    }

    pub fn parse_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "npm" | "node" | "yarn" | "pnpm" => Some(Self::Npm),
            "cargo" | "rust" | "crates" | "crates.io" => Some(Self::Cargo),
            "pip" | "python" | "pypi" => Some(Self::Pip),
            "maven" | "gradle" | "java" => Some(Self::Maven),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedRange {
    pub min_prot: u64,
    pub max_prot: u64,
    pub arch: u64,
    /// Inclusive: the earliest published `arch.feat.fix` with PROT in range.
    pub lower_bound: String,
    /// Exclusive: the next published `arch.feat.fix` with PROT out of range,
    /// when one is known. `None` means the range is open-ended.
    pub upper_bound: Option<String>,
    pub matched_versions: usize,
}

/// Resolves `[min_prot, max_prot]` against `manifest`'s history (filtered to
/// its current ARCH — a different ARCH is a different, incompatible major
/// line and never a sensible target for the same range).
pub fn resolve_range(manifest: &Manifest, min_prot: u64, max_prot: u64) -> Result<ResolvedRange> {
    if min_prot > max_prot {
        bail!("--min-prot ({min_prot}) must be <= --max-prot ({max_prot})");
    }

    let arch = manifest.identity.arch;
    let mut checkpoints: Vec<(u64, u64, u64, u64)> = manifest
        .history
        .iter()
        .filter(|entry| entry.arch == arch)
        .map(|entry| (entry.arch, entry.feat, entry.prot, entry.fix))
        .collect();

    let current = (
        manifest.identity.arch,
        manifest.identity.feat,
        manifest.identity.prot,
        manifest.identity.fix,
    );
    if checkpoints.last() != Some(&current) {
        checkpoints.push(current);
    }

    if checkpoints.is_empty() {
        bail!("manifest has no history entries to derive a version range from; run `generate` at least once");
    }

    let matching_indices: Vec<usize> = checkpoints
        .iter()
        .enumerate()
        .filter(|(_, checkpoint)| checkpoint.2 >= min_prot && checkpoint.2 <= max_prot)
        .map(|(index, _)| index)
        .collect();

    let Some(&first_idx) = matching_indices.first() else {
        bail!(
            "no recorded version has PROT in [{min_prot}, {max_prot}] for ARCH {arch}; recorded \
             PROT values were: {}",
            checkpoints
                .iter()
                .map(|checkpoint| checkpoint.2.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    };
    let last_idx = *matching_indices.last().expect("non-empty checked above");

    let lower = checkpoints[first_idx];
    let upper_bound = checkpoints
        .get(last_idx + 1)
        .map(|checkpoint| Identity::semver_projection(checkpoint.0, checkpoint.1, checkpoint.3));

    Ok(ResolvedRange {
        min_prot,
        max_prot,
        arch,
        lower_bound: Identity::semver_projection(lower.0, lower.1, lower.3),
        upper_bound,
        matched_versions: matching_indices.len(),
    })
}

/// Formats `range` as the native dependency-constraint syntax for `ecosystem`.
pub fn format_for_ecosystem(ecosystem: Ecosystem, range: &ResolvedRange) -> String {
    match (ecosystem, &range.upper_bound) {
        (Ecosystem::Npm, Some(upper)) => format!(">={} <{}", range.lower_bound, upper),
        (Ecosystem::Npm, None) => format!(">={}", range.lower_bound),
        (Ecosystem::Cargo, Some(upper)) => format!(">={}, <{}", range.lower_bound, upper),
        (Ecosystem::Cargo, None) => format!(">={}", range.lower_bound),
        (Ecosystem::Pip, Some(upper)) => format!(">={},<{}", range.lower_bound, upper),
        (Ecosystem::Pip, None) => format!(">={}", range.lower_bound),
        (Ecosystem::Maven, Some(upper)) => format!("[{},{})", range.lower_bound, upper),
        (Ecosystem::Maven, None) => format!("[{},)", range.lower_bound),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvs::manifest::{HistoryEntry, Identity, Manifest};

    fn history_entry(arch: u64, feat: u64, prot: u64, fix: u64) -> HistoryEntry {
        HistoryEntry {
            mvs: Identity::format_mvs(arch, feat, prot, fix, "cli"),
            arch,
            feat,
            prot,
            fix,
            cont: "cli".to_string(),
            reasons: vec!["test".to_string()],
            changed_at_unix: 0,
        }
    }

    fn manifest_with_history(entries: Vec<HistoryEntry>) -> Manifest {
        let mut manifest = Manifest::default_for_context("cli");
        let last = entries.last().expect("at least one entry");
        manifest.identity.arch = last.arch;
        manifest.identity.feat = last.feat;
        manifest.identity.prot = last.prot;
        manifest.identity.fix = last.fix;
        manifest.sync_identity_string();
        manifest.history = entries;
        manifest
    }

    #[test]
    fn resolves_inclusive_lower_and_exclusive_upper_bound() {
        // PROT: 0 at 1.0.0, 1 at 1.0.1, 1 at 1.1.0, 2 at 1.1.1 (current).
        let manifest = manifest_with_history(vec![
            history_entry(1, 0, 0, 0),
            history_entry(1, 0, 1, 1),
            history_entry(1, 1, 1, 0),
            history_entry(1, 1, 2, 1),
        ]);

        let range = resolve_range(&manifest, 1, 1).unwrap();
        assert_eq!(range.lower_bound, "1.0.1");
        assert_eq!(range.upper_bound.as_deref(), Some("1.1.1"));
        assert_eq!(range.matched_versions, 2);
    }

    #[test]
    fn open_ended_when_the_matching_range_reaches_the_current_version() {
        let manifest = manifest_with_history(vec![
            history_entry(1, 0, 0, 0),
            history_entry(1, 0, 1, 1),
            history_entry(1, 1, 1, 0),
        ]);

        let range = resolve_range(&manifest, 1, 1).unwrap();
        assert_eq!(range.lower_bound, "1.0.1");
        assert_eq!(range.upper_bound, None);
    }

    #[test]
    fn different_arch_history_is_ignored() {
        let manifest =
            manifest_with_history(vec![history_entry(1, 0, 5, 0), history_entry(2, 0, 0, 0)]);
        // Current ARCH is 2 (last entry); ARCH-1 PROT-5 history must not be considered.
        let error = resolve_range(&manifest, 5, 5).unwrap_err();
        assert!(error.to_string().contains("no recorded version"));
    }

    #[test]
    fn rejects_an_inverted_range() {
        let manifest = manifest_with_history(vec![history_entry(1, 0, 0, 0)]);
        let error = resolve_range(&manifest, 5, 1).unwrap_err();
        assert!(error.to_string().contains("must be <="));
    }

    #[test]
    fn formats_each_ecosystem_with_and_without_an_upper_bound() {
        let bounded = ResolvedRange {
            min_prot: 1,
            max_prot: 1,
            arch: 1,
            lower_bound: "1.0.1".to_string(),
            upper_bound: Some("1.1.1".to_string()),
            matched_versions: 2,
        };
        assert_eq!(
            format_for_ecosystem(Ecosystem::Npm, &bounded),
            ">=1.0.1 <1.1.1"
        );
        assert_eq!(
            format_for_ecosystem(Ecosystem::Cargo, &bounded),
            ">=1.0.1, <1.1.1"
        );
        assert_eq!(
            format_for_ecosystem(Ecosystem::Pip, &bounded),
            ">=1.0.1,<1.1.1"
        );
        assert_eq!(
            format_for_ecosystem(Ecosystem::Maven, &bounded),
            "[1.0.1,1.1.1)"
        );

        let open = ResolvedRange {
            upper_bound: None,
            ..bounded
        };
        assert_eq!(format_for_ecosystem(Ecosystem::Npm, &open), ">=1.0.1");
        assert_eq!(format_for_ecosystem(Ecosystem::Cargo, &open), ">=1.0.1");
        assert_eq!(format_for_ecosystem(Ecosystem::Pip, &open), ">=1.0.1");
        assert_eq!(format_for_ecosystem(Ecosystem::Maven, &open), "[1.0.1,)");
    }
}
