// SPDX-License-Identifier: AGPL-3.0-only
//! Domain logic behind `mvs-manager migrate`: detecting what a project's
//! current versioning setup looks like, planning a proposed `mvs.json`,
//! replaying git tag history to check whether past releases would have
//! honestly needed a PROT bump, and the on-disk snapshot format `apply`/
//! `rollback` use to make migration reversible.
//!
//! This module has no CLI dependency — it operates purely on paths and
//! returns data; `src/commands/migrate.rs` handles argument parsing, output
//! rendering, and the file-writing side effects of `apply`/`rollback`.

pub mod importers;

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::mvs::crawler::crawl_codebase;
use crate::mvs::hashing::hash_items;
use crate::mvs::manifest::{
    Evidence, Identity, Manifest, ProtocolRange, PublicApiSnapshot, ScanPolicy, VersionFileEntry,
    VersionFileKind, VersionProjection,
};
use crate::mvs::project_detect::{self, ProjectDetection};
use crate::mvs::schemes::{self, AxisMapping, LegacyVersion, MvsAxes, VersionScheme};
use crate::mvs::vcs;
use crate::mvs::version_sources;
use importers::ReleaseToolingHit;

// ── Detection ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct VersionSourceStatus {
    pub path: String,
    pub kind: VersionFileKind,
    pub current_version: String,
}

#[derive(Debug)]
pub struct DetectionResult {
    pub git_available: bool,
    pub is_git_repo: bool,
    /// Chronological, oldest first.
    pub tags: Vec<String>,
    pub latest_tag: Option<String>,
    pub version_sources: Vec<VersionSourceStatus>,
    pub version_sources_agree: bool,
    pub release_tooling: Vec<ReleaseToolingHit>,
    pub project: ProjectDetection,
}

pub fn gather_detection(root: &Path) -> DetectionResult {
    let git_available = vcs::is_available();
    let is_git_repo = git_available && vcs::is_repo(root);
    let tags = if is_git_repo {
        vcs::list_tags_chronological(root).unwrap_or_default()
    } else {
        Vec::new()
    };
    let latest_tag = tags.last().cloned();

    let mut version_sources_status = Vec::new();
    for entry in version_sources::detect(root) {
        if let Ok(current_version) = version_sources::read_version(root, &entry) {
            version_sources_status.push(VersionSourceStatus {
                path: entry.path,
                kind: entry.kind,
                current_version,
            });
        }
    }
    let version_sources_agree = version_sources_status
        .windows(2)
        .all(|pair| pair[0].current_version == pair[1].current_version);

    let release_tooling = importers::detect(root);
    let project = project_detect::detect_project(root).unwrap_or_else(|_| ProjectDetection {
        languages: Default::default(),
        markers: Vec::new(),
    });

    DetectionResult {
        git_available,
        is_git_repo,
        tags,
        latest_tag,
        version_sources: version_sources_status,
        version_sources_agree,
        release_tooling,
        project,
    }
}

// ── Plan ──────────────────────────────────────────────────────────────────

pub struct PlanOptions<'a> {
    pub context: &'a str,
    pub scheme: Option<VersionScheme>,
    pub scheme_regex: Option<&'a str>,
    pub map_spec: Option<&'a str>,
    pub from_version: Option<&'a str>,
    pub preset: Option<&'a str>,
    pub prot: Option<u64>,
    pub allow_non_monotonic: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TagPreviewRow {
    pub tag: String,
    pub parsed: bool,
    pub mvs_identity: Option<String>,
}

#[derive(Debug)]
pub struct PlanResult {
    pub source_version: String,
    pub source_label: &'static str,
    pub legacy: LegacyVersion,
    pub mapping_source: &'static str,
    pub axes: MvsAxes,
    pub manifest: Manifest,
    pub tag_preview: Vec<TagPreviewRow>,
    pub invariant_ok: bool,
    pub invariant_message: Option<String>,
    pub advisories: Vec<String>,
    /// Files an importer (bumpversion/tbump) named as carrying the version,
    /// but whose format isn't one of the known `VersionFileKind`s — surfaced
    /// for visibility rather than guessed at and possibly corrupted by `sync`.
    pub unrecognized_imported_version_files: Vec<String>,
}

pub fn build_plan(
    root: &Path,
    detection: &DetectionResult,
    options: &PlanOptions<'_>,
) -> Result<PlanResult> {
    let (source_version, source_label) = if let Some(v) = options.from_version {
        (v.to_string(), "from_version_override")
    } else if let Some(tag) = &detection.latest_tag {
        (tag.clone(), "latest_git_tag")
    } else if let Some(first) = detection.version_sources.first() {
        (first.current_version.clone(), "version_source")
    } else {
        bail!(
            "could not determine a version to migrate from (no git tags, no recognized version \
             file); pass --from-version explicitly"
        );
    };

    let legacy = match options.scheme {
        Some(scheme) => schemes::parse(&source_version, scheme, options.scheme_regex)?,
        None => schemes::parse_auto(&source_version)?,
    };

    let (mapping, mapping_source) = match options.map_spec {
        Some(spec) => (AxisMapping::parse_spec(spec)?, "custom"),
        None => (schemes::default_mapping_for(legacy.scheme), "default"),
    };

    let mut axes = schemes::resolve_axes(&legacy, &mapping);
    if let Some(prot) = options.prot {
        axes.prot = prot;
    }

    let (invariant_ok, invariant_message) = check_monotonic_invariant(
        detection,
        legacy.scheme,
        options.scheme_regex,
        &mapping,
        &axes,
    );
    if !invariant_ok && !options.allow_non_monotonic {
        bail!(invariant_message.unwrap_or_else(|| "proposed version is not monotonic".to_string()));
    }

    let tag_preview = detection
        .tags
        .iter()
        .map(
            |tag| match schemes::parse(tag, legacy.scheme, options.scheme_regex) {
                Ok(tag_legacy) => {
                    let tag_axes = schemes::resolve_axes(&tag_legacy, &mapping);
                    TagPreviewRow {
                        tag: tag.clone(),
                        parsed: true,
                        mvs_identity: Some(Identity::format_mvs(
                            tag_axes.arch,
                            tag_axes.feat,
                            tag_axes.prot,
                            tag_axes.fix,
                            options.context,
                        )),
                    }
                }
                Err(_) => TagPreviewRow {
                    tag: tag.clone(),
                    parsed: false,
                    mvs_identity: None,
                },
            },
        )
        .collect();

    let mut manifest = Manifest::default_for_context(options.context);
    manifest.identity.arch = axes.arch;
    manifest.identity.feat = axes.feat;
    manifest.identity.prot = axes.prot;
    manifest.identity.fix = axes.fix;
    manifest.sync_identity_string();
    manifest.compatibility.host_range = ProtocolRange {
        min_prot: axes.prot,
        max_prot: axes.prot,
    };
    manifest.compatibility.extension_range = ProtocolRange {
        min_prot: axes.prot,
        max_prot: axes.prot,
    };
    manifest.scan_policy =
        project_detect::build_scan_policy(root, &detection.project, options.preset);

    let mut version_files: Vec<VersionFileEntry> = detection
        .version_sources
        .iter()
        .map(|status| VersionFileEntry {
            path: status.path.clone(),
            kind: status.kind,
            projection: VersionProjection::default(),
        })
        .collect();

    let mut unrecognized_imported_version_files = Vec::new();
    for hit in &detection.release_tooling {
        for path in &hit.imported_version_files {
            if version_files.iter().any(|f| &f.path == path) {
                continue;
            }
            match guess_kind_for_path(root, path) {
                Some(kind) => version_files.push(VersionFileEntry {
                    path: path.clone(),
                    kind,
                    projection: VersionProjection::default(),
                }),
                None => unrecognized_imported_version_files.push(path.clone()),
            }
        }
    }
    manifest.release.version_files = version_files;

    manifest.append_history_entry(vec![format!(
        "Migrated from {} `{}` (scheme: {}, mapping: {}).",
        source_label.replace('_', " "),
        source_version,
        legacy.scheme.as_str(),
        mapping_source
    )]);

    let advisories = schemes::advisories(&legacy, &mapping);

    Ok(PlanResult {
        source_version,
        source_label,
        legacy,
        mapping_source,
        axes,
        manifest,
        tag_preview,
        invariant_ok,
        invariant_message,
        advisories,
        unrecognized_imported_version_files,
    })
}

/// Invariant: the proposed SemVer projection must never be lower than the
/// latest published tag's, so a migration can't accidentally make a
/// registry go backwards.
fn check_monotonic_invariant(
    detection: &DetectionResult,
    scheme: VersionScheme,
    scheme_regex: Option<&str>,
    mapping: &AxisMapping,
    proposed: &MvsAxes,
) -> (bool, Option<String>) {
    let Some(tag) = &detection.latest_tag else {
        return (true, None);
    };
    let Ok(tag_legacy) = schemes::parse(tag, scheme, scheme_regex) else {
        return (true, None);
    };
    let published = schemes::resolve_axes(&tag_legacy, mapping);

    let proposed_semver = (proposed.arch, proposed.feat, proposed.fix);
    let published_semver = (published.arch, published.feat, published.fix);

    if proposed_semver >= published_semver {
        (true, None)
    } else {
        (
            false,
            Some(format!(
                "proposed SemVer projection {}.{}.{} is lower than the latest published tag \
                 `{tag}`'s projection {}.{}.{}; pass --from-version to pick a version at or \
                 above it, or --allow-non-monotonic to override",
                proposed.arch,
                proposed.feat,
                proposed.fix,
                published.arch,
                published.feat,
                published.fix
            )),
        )
    }
}

fn guess_kind_for_path(root: &Path, path: &str) -> Option<VersionFileKind> {
    let basename = Path::new(path).file_name()?.to_str()?;
    match basename {
        "Cargo.toml" => Some(VersionFileKind::CargoToml),
        "package.json" => Some(VersionFileKind::NpmPackageJson),
        "composer.json" => Some(VersionFileKind::ComposerJson),
        "pubspec.yaml" => Some(VersionFileKind::PubspecYaml),
        "gradle.properties" => Some(VersionFileKind::GradleProperties),
        "build.gradle" | "build.gradle.kts" => Some(VersionFileKind::BuildGradle),
        "pom.xml" => Some(VersionFileKind::PomXml),
        "pyproject.toml" => {
            let content = fs::read_to_string(root.join(path)).ok()?;
            if content.contains("[tool.poetry]") {
                Some(VersionFileKind::PyprojectPoetry)
            } else {
                Some(VersionFileKind::PyprojectPep621)
            }
        }
        _ => match Path::new(path).extension().and_then(|ext| ext.to_str()) {
            Some("csproj") => Some(VersionFileKind::Csproj),
            Some("gemspec") => Some(VersionFileKind::GemspecLiteral),
            Some("rockspec") => Some(VersionFileKind::LuaRockspec),
            _ => None,
        },
    }
}

// ── Backfill ──────────────────────────────────────────────────────────────

pub struct BackfillOptions<'a> {
    /// Explicit tags, in the order to process. When `None`, the most recent
    /// `limit` tags (chronological) are used.
    pub tags: Option<Vec<String>>,
    pub limit: usize,
    pub scheme: Option<VersionScheme>,
    pub scheme_regex: Option<&'a str>,
    pub map_spec: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BumpLevel {
    Major,
    Minor,
    Patch,
    None,
    Unknown,
}

impl BumpLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            BumpLevel::Major => "major",
            BumpLevel::Minor => "minor",
            BumpLevel::Patch => "patch",
            BumpLevel::None => "none",
            BumpLevel::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TransitionReport {
    pub from_tag: String,
    pub to_tag: String,
    pub bump_level: BumpLevel,
    pub features_added: usize,
    pub features_removed: usize,
    pub protocols_added: usize,
    pub protocols_removed: usize,
    pub public_api_added: usize,
    pub public_api_removed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub violation: Option<String>,
}

#[derive(Debug)]
pub struct BackfillResult {
    pub resolved_scheme: Option<VersionScheme>,
    pub tags_considered: Vec<String>,
    pub skipped_tags: Vec<(String, String)>,
    pub transitions: Vec<TransitionReport>,
    pub violation_count: usize,
}

/// Replays git tag history through the crawler to see whether a breaking
/// public API/protocol change ever shipped as a patch (which should carry
/// zero surface change) or a minor release removed surface (a breaking
/// change under conventional SemVer, even though additions in a minor are
/// fine). Requires `git` and a git repository; each tag is checked out into
/// a disposable worktree, crawled, and immediately removed.
pub fn run_backfill(
    root: &Path,
    scan_policy: &ScanPolicy,
    options: &BackfillOptions<'_>,
) -> Result<BackfillResult> {
    if !vcs::is_available() {
        bail!("git is not available on PATH; `migrate backfill` requires it");
    }
    if !vcs::is_repo(root) {
        bail!("`{}` is not a git repository", root.display());
    }

    let all_tags = vcs::list_tags_chronological(root)?;
    if all_tags.is_empty() {
        bail!("no git tags found to backfill from");
    }

    let tags: Vec<String> = match &options.tags {
        Some(explicit) => explicit.clone(),
        None => {
            let take = options.limit.min(all_tags.len());
            all_tags[all_tags.len() - take..].to_vec()
        }
    };
    if tags.len() < 2 {
        bail!(
            "backfill needs at least 2 tags to compare transitions; got {} (adjust --tags/--limit)",
            tags.len()
        );
    }

    let resolved_scheme = match options.scheme {
        Some(scheme) => Some(scheme),
        None => tags
            .iter()
            .find_map(|tag| schemes::parse_auto(tag).ok().map(|legacy| legacy.scheme)),
    };
    let mapping = match (options.map_spec, resolved_scheme) {
        (Some(spec), _) => Some(AxisMapping::parse_spec(spec)?),
        (None, Some(scheme)) => Some(schemes::default_mapping_for(scheme)),
        (None, None) => None,
    };

    let mut snapshots: Vec<(String, Evidence)> = Vec::with_capacity(tags.len());
    let mut skipped_tags = Vec::new();
    for tag in &tags {
        match crawl_tag(root, tag, scan_policy) {
            Ok(evidence) => snapshots.push((tag.clone(), evidence)),
            Err(error) => skipped_tags.push((tag.clone(), format!("{error:#}"))),
        }
    }

    let mut transitions = Vec::with_capacity(snapshots.len().saturating_sub(1));
    let mut violation_count = 0;

    for pair in snapshots.windows(2) {
        let (from_tag, from_evidence) = &pair[0];
        let (to_tag, to_evidence) = &pair[1];

        let diff = from_evidence.semantic_diff(
            &to_evidence.feature_inventory,
            &to_evidence.protocol_inventory,
            &to_evidence.public_api_inventory,
        );

        let bump_level = match (resolved_scheme, &mapping) {
            (Some(scheme), Some(mapping)) => match (
                schemes::parse(from_tag, scheme, options.scheme_regex),
                schemes::parse(to_tag, scheme, options.scheme_regex),
            ) {
                (Ok(from_legacy), Ok(to_legacy)) => classify_bump(
                    &schemes::resolve_axes(&from_legacy, mapping),
                    &schemes::resolve_axes(&to_legacy, mapping),
                ),
                _ => BumpLevel::Unknown,
            },
            _ => BumpLevel::Unknown,
        };

        let surface_added = diff.protocols.added.len() + diff.public_api.added.len();
        let surface_removed = diff.protocols.removed.len() + diff.public_api.removed.len();

        let violation = match bump_level {
            BumpLevel::Patch if surface_added + surface_removed > 0 => Some(format!(
                "`{to_tag}` was published as a patch-level release but the public API/protocol \
                 surface changed ({surface_added} added, {surface_removed} removed); patch \
                 releases should carry zero surface change."
            )),
            BumpLevel::Minor if surface_removed > 0 => Some(format!(
                "`{to_tag}` was published as a minor-level release but removed {surface_removed} \
                 public API/protocol item(s); that's a breaking change under conventional SemVer."
            )),
            _ => None,
        };
        if violation.is_some() {
            violation_count += 1;
        }

        transitions.push(TransitionReport {
            from_tag: from_tag.clone(),
            to_tag: to_tag.clone(),
            bump_level,
            features_added: diff.features.added.len(),
            features_removed: diff.features.removed.len(),
            protocols_added: diff.protocols.added.len(),
            protocols_removed: diff.protocols.removed.len(),
            public_api_added: diff.public_api.added.len(),
            public_api_removed: diff.public_api.removed.len(),
            violation,
        });
    }

    Ok(BackfillResult {
        resolved_scheme,
        tags_considered: tags,
        skipped_tags,
        transitions,
        violation_count,
    })
}

fn crawl_tag(root: &Path, tag: &str, scan_policy: &ScanPolicy) -> Result<Evidence> {
    let worktree = vcs::Worktree::create(root, tag)?;
    let report = crawl_codebase(&worktree.path, scan_policy)
        .with_context(|| format!("failed to crawl tag `{tag}`"))?;

    let feature_inventory: Vec<String> = report.feature_tags.iter().cloned().collect();
    let protocol_inventory: Vec<String> = report.protocol_tags.iter().cloned().collect();
    let mut raw_public_api: Vec<PublicApiSnapshot> = report
        .public_api
        .iter()
        .map(|signature| PublicApiSnapshot {
            file: signature.file.clone(),
            signature: signature.signature.clone(),
        })
        .collect();
    raw_public_api.sort();
    raw_public_api.dedup();
    let (public_api_inventory, public_api_hash) =
        Evidence::canonicalize_public_api_inventory(raw_public_api);

    Ok(Evidence {
        feature_hash: hash_items(feature_inventory.iter().map(String::as_str)),
        protocol_hash: hash_items(protocol_inventory.iter().map(String::as_str)),
        public_api_hash,
        feature_inventory,
        protocol_inventory,
        public_api_inventory,
    })
}

fn classify_bump(from: &MvsAxes, to: &MvsAxes) -> BumpLevel {
    if to.arch > from.arch {
        BumpLevel::Major
    } else if to.arch < from.arch {
        BumpLevel::Unknown
    } else if to.feat > from.feat {
        BumpLevel::Minor
    } else if to.feat < from.feat {
        BumpLevel::Unknown
    } else if to.fix > from.fix {
        BumpLevel::Patch
    } else if to.fix < from.fix {
        BumpLevel::Unknown
    } else {
        BumpLevel::None
    }
}

// ── Snapshot (apply/rollback) ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionFileBackup {
    pub path: String,
    pub previous_contents: String,
}

/// What `migrate apply` writes to `.mvs/migration-snapshot.json` before
/// touching anything, so `migrate rollback` can restore exactly what was
/// there.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationSnapshot {
    pub created_at_unix: u64,
    pub manifest_path: String,
    /// `None` when `mvs.json` did not exist before `apply` (so rollback
    /// deletes it rather than restoring empty content).
    pub previous_manifest_contents: Option<String>,
    pub version_files: Vec<VersionFileBackup>,
}

impl MigrationSnapshot {
    pub fn path(root: &Path) -> PathBuf {
        root.join(".mvs").join("migration-snapshot.json")
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let path = Self::path(root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create `{}`", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self).context("failed to serialize snapshot")?;
        fs::write(&path, format!("{json}\n"))
            .with_context(|| format!("failed to write snapshot to `{}`", path.display()))
    }

    pub fn load(root: &Path) -> Result<Self> {
        let path = Self::path(root);
        let raw = fs::read_to_string(&path).with_context(|| {
            format!(
                "no migration snapshot found at `{}`; nothing to roll back",
                path.display()
            )
        })?;
        serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse migration snapshot at `{}`", path.display()))
    }

    pub fn remove(root: &Path) -> Result<()> {
        let path = Self::path(root);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove snapshot at `{}`", path.display()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvs::manifest::VersionFileKind;
    use std::fs;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "mvs-migrate-test-{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn classify_bump_orders_axes_major_over_minor_over_patch() {
        let base = MvsAxes {
            arch: 1,
            feat: 2,
            prot: 0,
            fix: 3,
        };
        assert_eq!(
            classify_bump(&base, &MvsAxes { arch: 2, ..base }),
            BumpLevel::Major
        );
        assert_eq!(
            classify_bump(&base, &MvsAxes { feat: 3, ..base }),
            BumpLevel::Minor
        );
        assert_eq!(
            classify_bump(&base, &MvsAxes { fix: 4, ..base }),
            BumpLevel::Patch
        );
        assert_eq!(classify_bump(&base, &base), BumpLevel::None);
        assert_eq!(
            classify_bump(&base, &MvsAxes { arch: 0, ..base }),
            BumpLevel::Unknown
        );
    }

    #[test]
    fn guess_kind_for_path_recognizes_known_basenames_and_sniffs_pyproject() {
        let dir = TempDir::new("guess-kind");
        fs::write(
            dir.path().join("pyproject.toml"),
            "[tool.poetry]\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        assert_eq!(
            guess_kind_for_path(dir.path(), "Cargo.toml"),
            Some(VersionFileKind::CargoToml)
        );
        assert_eq!(
            guess_kind_for_path(dir.path(), "pyproject.toml"),
            Some(VersionFileKind::PyprojectPoetry)
        );
        assert_eq!(guess_kind_for_path(dir.path(), "README.md"), None);
        assert_eq!(
            guess_kind_for_path(dir.path(), "nuget/MyLib.csproj"),
            Some(VersionFileKind::Csproj)
        );
    }

    #[test]
    fn migration_snapshot_round_trips_through_save_load_remove() {
        let dir = TempDir::new("snapshot");
        let snapshot = MigrationSnapshot {
            created_at_unix: 12345,
            manifest_path: "mvs.json".to_string(),
            previous_manifest_contents: Some("{}".to_string()),
            version_files: vec![VersionFileBackup {
                path: "Cargo.toml".to_string(),
                previous_contents: "[package]\nversion = \"1.0.0\"\n".to_string(),
            }],
        };
        snapshot.save(dir.path()).unwrap();
        assert!(MigrationSnapshot::path(dir.path()).exists());

        let loaded = MigrationSnapshot::load(dir.path()).unwrap();
        assert_eq!(loaded.created_at_unix, 12345);
        assert_eq!(loaded.version_files.len(), 1);

        MigrationSnapshot::remove(dir.path()).unwrap();
        assert!(!MigrationSnapshot::path(dir.path()).exists());
        assert!(MigrationSnapshot::load(dir.path()).is_err());
    }

    #[test]
    fn detection_reports_no_repo_and_no_sources_in_a_bare_directory() {
        let dir = TempDir::new("bare-detect");
        let detection = gather_detection(dir.path());
        assert!(!detection.is_git_repo);
        assert!(detection.tags.is_empty());
        assert!(detection.version_sources.is_empty());
        assert!(detection.version_sources_agree);
    }

    #[test]
    fn plan_fails_clearly_with_no_version_source_at_all() {
        let dir = TempDir::new("plan-no-source");
        let detection = gather_detection(dir.path());
        let options = PlanOptions {
            context: "cli",
            scheme: None,
            scheme_regex: None,
            map_spec: None,
            from_version: None,
            preset: None,
            prot: None,
            allow_non_monotonic: false,
        };
        let error = build_plan(dir.path(), &detection, &options).unwrap_err();
        assert!(error.to_string().contains("could not determine a version"));
    }

    #[test]
    fn plan_converts_from_version_override_into_a_proposed_manifest() {
        let dir = TempDir::new("plan-override");
        let detection = gather_detection(dir.path());
        let options = PlanOptions {
            context: "cli",
            scheme: None,
            scheme_regex: None,
            map_spec: None,
            from_version: Some("2.4.1"),
            preset: None,
            prot: Some(3),
            allow_non_monotonic: false,
        };
        let plan = build_plan(dir.path(), &detection, &options).unwrap();
        assert_eq!(plan.source_label, "from_version_override");
        assert_eq!(plan.axes.arch, 2);
        assert_eq!(plan.axes.feat, 4);
        assert_eq!(plan.axes.prot, 3);
        assert_eq!(plan.axes.fix, 1);
        assert_eq!(plan.manifest.identity.mvs, "2.4.3.1-cli");
        assert_eq!(plan.manifest.history.len(), 1);
    }

    fn run_git(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .unwrap_or_else(|error| panic!("failed to run git {args:?}: {error}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Three tags over one file: v1.0.0 ships a protocol surface, v1.0.1
    /// (patch) removes it (a genuine violation), v1.1.0 (minor) adds it back
    /// (a fine addition, not a violation).
    fn init_backfill_repo(dir: &Path) {
        run_git(dir, &["init", "-q", "-b", "main"]);

        fs::write(
            dir.join("lib.rs"),
            "/// @mvs-protocol(\"core\")\npub fn core_fn() {}\n",
        )
        .unwrap();
        run_git(dir, &["add", "-A"]);
        run_git(dir, &["commit", "-q", "-m", "v1.0.0"]);
        run_git(dir, &["tag", "v1.0.0"]);

        fs::write(dir.join("lib.rs"), "pub fn core_fn() {}\n").unwrap();
        run_git(dir, &["add", "-A"]);
        run_git(dir, &["commit", "-q", "-m", "v1.0.1"]);
        run_git(dir, &["tag", "v1.0.1"]);

        fs::write(
            dir.join("lib.rs"),
            "/// @mvs-protocol(\"core\")\npub fn core_fn() {}\n",
        )
        .unwrap();
        run_git(dir, &["add", "-A"]);
        run_git(dir, &["commit", "-q", "-m", "v1.1.0"]);
        run_git(dir, &["tag", "v1.1.0"]);
    }

    #[test]
    fn backfill_flags_patch_that_removed_surface_but_not_minor_that_added_it_back() {
        if !vcs::is_available() {
            eprintln!("skipping: git not available");
            return;
        }
        let dir = TempDir::new("backfill");
        init_backfill_repo(dir.path());

        let scan_policy = ScanPolicy::default();
        let options = BackfillOptions {
            tags: None,
            limit: 10,
            scheme: None,
            scheme_regex: None,
            map_spec: None,
        };
        let result = run_backfill(dir.path(), &scan_policy, &options).unwrap();

        assert_eq!(result.tags_considered, vec!["v1.0.0", "v1.0.1", "v1.1.0"]);
        assert!(result.skipped_tags.is_empty());
        assert_eq!(result.transitions.len(), 2);

        let patch_transition = &result.transitions[0];
        assert_eq!(patch_transition.from_tag, "v1.0.0");
        assert_eq!(patch_transition.to_tag, "v1.0.1");
        assert_eq!(patch_transition.bump_level, BumpLevel::Patch);
        assert_eq!(patch_transition.protocols_removed, 1);
        assert!(patch_transition.violation.is_some());

        let minor_transition = &result.transitions[1];
        assert_eq!(minor_transition.from_tag, "v1.0.1");
        assert_eq!(minor_transition.to_tag, "v1.1.0");
        assert_eq!(minor_transition.bump_level, BumpLevel::Minor);
        assert_eq!(minor_transition.protocols_added, 1);
        assert!(minor_transition.violation.is_none());

        assert_eq!(result.violation_count, 1);
    }
}
