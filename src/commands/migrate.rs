// SPDX-License-Identifier: AGPL-3.0-only
use std::{fs, path::PathBuf};

use serde::Serialize;

use crate::cli::{
    MigrateAction, MigrateApplyArgs, MigrateArgs, MigrateBackfillArgs, MigrateDetectArgs,
    MigratePlanArgs, MigrateRollbackArgs, MigrationConversionArgs, OutputFormat,
    EXIT_MIGRATE_ERROR, EXIT_SUCCESS,
};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::mvs::manifest::VersionFileEntry;
use crate::mvs::migrate::{self, BackfillOptions, DetectionResult, PlanOptions, PlanResult};
use crate::mvs::schemes::{self, VersionScheme};
use crate::mvs::version_sources;

/// @mvs-feature("versioning_migration")
/// @mvs-protocol("cli_migrate_command")
pub fn run(args: MigrateArgs) -> i32 {
    match args.action {
        MigrateAction::Detect(args) => run_detect(args),
        MigrateAction::Plan(args) => run_plan(args),
        MigrateAction::Backfill(args) => run_backfill(args),
        MigrateAction::Apply(args) => run_apply(args),
        MigrateAction::Rollback(args) => run_rollback(args),
    }
}

fn resolve_scheme(name: Option<&str>) -> Result<Option<VersionScheme>, CommandFailure> {
    match name {
        None => Ok(None),
        Some(name) => VersionScheme::parse_name(name).map(Some).ok_or_else(|| {
            CommandFailure::new(
                EXIT_MIGRATE_ERROR,
                format!(
                    "unknown --scheme `{name}`; expected one of semver, zerover, pep440, \
                     maven-gradle, dotnet4, go-modules, calver, integer, debian-rpm, custom"
                ),
            )
        }),
    }
}

fn current_unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── detect ────────────────────────────────────────────────────────────────

fn run_detect(args: MigrateDetectArgs) -> i32 {
    let detection = migrate::gather_detection(&args.root);
    let report = detect_report(&args.root.display().to_string(), &detection);
    match render_json_or(&report, args.format, render_detect_text) {
        Ok(()) => EXIT_SUCCESS,
        Err(error) => emit_error("migrate", args.format, error.exit_code, &error.message),
    }
}

fn detect_report(root: &str, detection: &DetectionResult) -> DetectReport {
    let detected_scheme = detection
        .latest_tag
        .as_deref()
        .or_else(|| {
            detection
                .version_sources
                .first()
                .map(|s| s.current_version.as_str())
        })
        .and_then(|version| schemes::parse_auto(version).ok())
        .map(|legacy| legacy.scheme.as_str());

    DetectReport {
        command: "migrate",
        subcommand: "detect",
        root: root.to_string(),
        git_available: detection.git_available,
        is_git_repo: detection.is_git_repo,
        tag_count: detection.tags.len(),
        latest_tag: detection.latest_tag.clone(),
        detected_scheme,
        version_sources: detection
            .version_sources
            .iter()
            .map(|status| VersionSourcePayload {
                path: status.path.clone(),
                kind: status.kind.as_str(),
                current_version: status.current_version.clone(),
            })
            .collect(),
        version_sources_agree: detection.version_sources_agree,
        release_tooling: detection
            .release_tooling
            .iter()
            .map(|hit| ReleaseToolingPayload {
                tool: hit.tool,
                config_path: hit.config_path.clone(),
                imported_version_files: hit.imported_version_files.clone(),
            })
            .collect(),
        detected_languages: detection.project.languages.iter().copied().collect(),
    }
}

fn render_detect_text(report: &DetectReport) -> Result<(), CommandFailure> {
    println!("migrate detect: {}", report.root);
    println!(
        "  git: available={}, repo={}, tags={}",
        report.git_available, report.is_git_repo, report.tag_count
    );
    if let Some(tag) = &report.latest_tag {
        println!("  latest tag: {tag}");
    }
    if let Some(scheme) = report.detected_scheme {
        println!("  detected scheme: {scheme}");
    }
    if report.version_sources.is_empty() {
        println!("  version sources: none detected");
    } else {
        println!(
            "  version sources ({}):",
            if report.version_sources_agree {
                "agree"
            } else {
                "DISAGREE"
            }
        );
        for source in &report.version_sources {
            println!(
                "    {} ({}): {}",
                source.path, source.kind, source.current_version
            );
        }
    }
    if !report.release_tooling.is_empty() {
        println!("  release tooling:");
        for hit in &report.release_tooling {
            if hit.imported_version_files.is_empty() {
                println!("    {} ({})", hit.tool, hit.config_path);
            } else {
                println!(
                    "    {} ({}): {}",
                    hit.tool,
                    hit.config_path,
                    hit.imported_version_files.join(", ")
                );
            }
        }
    }
    if !report.detected_languages.is_empty() {
        println!("  languages: {}", report.detected_languages.join(", "));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct DetectReport {
    command: &'static str,
    subcommand: &'static str,
    root: String,
    git_available: bool,
    is_git_repo: bool,
    tag_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detected_scheme: Option<&'static str>,
    version_sources: Vec<VersionSourcePayload>,
    version_sources_agree: bool,
    release_tooling: Vec<ReleaseToolingPayload>,
    detected_languages: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct VersionSourcePayload {
    path: String,
    kind: &'static str,
    current_version: String,
}

#[derive(Debug, Serialize)]
struct ReleaseToolingPayload {
    tool: &'static str,
    config_path: String,
    imported_version_files: Vec<String>,
}

// ── plan ──────────────────────────────────────────────────────────────────

fn run_plan(args: MigratePlanArgs) -> i32 {
    match try_plan(&args.common) {
        Ok(plan) => match render_json_or(
            &plan_report(&args.common.root.display().to_string(), &plan),
            args.format,
            render_plan_text,
        ) {
            Ok(()) => EXIT_SUCCESS,
            Err(error) => emit_error("migrate", args.format, error.exit_code, &error.message),
        },
        Err(error) => emit_error("migrate", args.format, error.exit_code, &error.message),
    }
}

fn try_plan(common: &MigrationConversionArgs) -> Result<PlanResult, CommandFailure> {
    let scheme = resolve_scheme(common.scheme.as_deref())?;
    let detection = migrate::gather_detection(&common.root);
    let options = PlanOptions {
        context: &common.context,
        scheme,
        scheme_regex: common.scheme_regex.as_deref(),
        map_spec: common.map.as_deref(),
        from_version: common.from_version.as_deref(),
        preset: common.preset.as_deref(),
        prot: common.prot,
        allow_non_monotonic: common.allow_non_monotonic,
    };
    migrate::build_plan(&common.root, &detection, &options)
        .map_err(|error| CommandFailure::new(EXIT_MIGRATE_ERROR, format!("{error:#}")))
}

fn plan_report(root: &str, plan: &PlanResult) -> PlanReportPayload {
    PlanReportPayload {
        command: "migrate",
        subcommand: "plan",
        root: root.to_string(),
        source_version: plan.source_version.clone(),
        source_label: plan.source_label,
        scheme: plan.legacy.scheme.as_str(),
        mapping_source: plan.mapping_source,
        axes: AxesPayload {
            arch: plan.axes.arch,
            feat: plan.axes.feat,
            prot: plan.axes.prot,
            fix: plan.axes.fix,
        },
        identity: plan.manifest.identity.mvs.clone(),
        semver_projection: plan.manifest.identity.package_semver(),
        invariant_ok: plan.invariant_ok,
        invariant_message: plan.invariant_message.clone(),
        version_files: plan
            .manifest
            .release
            .version_files
            .iter()
            .map(version_file_payload)
            .collect(),
        unrecognized_imported_version_files: plan.unrecognized_imported_version_files.clone(),
        advisories: plan.advisories.clone(),
        tag_preview: plan.tag_preview.clone(),
        manifest: plan.manifest.clone(),
    }
}

fn version_file_payload(entry: &VersionFileEntry) -> VersionFilePayload {
    VersionFilePayload {
        path: entry.path.clone(),
        kind: entry.kind.as_str(),
    }
}

fn render_plan_text(report: &PlanReportPayload) -> Result<(), CommandFailure> {
    println!(
        "migrate plan: {} `{}` ({}, scheme: {}, mapping: {})",
        report.source_label.replace('_', " "),
        report.source_version,
        report.root,
        report.scheme,
        report.mapping_source
    );
    println!(
        "  proposed identity: {} (ARCH {}, FEAT {}, PROT {}, FIX {})",
        report.identity, report.axes.arch, report.axes.feat, report.axes.prot, report.axes.fix
    );
    println!("  SemVer projection: {}", report.semver_projection);
    println!(
        "  monotonic invariant: {}",
        if report.invariant_ok {
            "ok"
        } else {
            "VIOLATED"
        }
    );
    if let Some(message) = &report.invariant_message {
        println!("    {message}");
    }
    if !report.version_files.is_empty() {
        println!("  release.version_files:");
        for file in &report.version_files {
            println!("    {} ({})", file.path, file.kind);
        }
    }
    if !report.unrecognized_imported_version_files.is_empty() {
        println!(
            "  imported but unrecognized (not added to release.version_files): {}",
            report.unrecognized_imported_version_files.join(", ")
        );
    }
    for advisory in &report.advisories {
        println!("  - {advisory}");
    }
    if !report.tag_preview.is_empty() {
        println!("  tag preview:");
        for row in &report.tag_preview {
            match &row.mvs_identity {
                Some(identity) => println!("    {} -> {identity}", row.tag),
                None => println!("    {} -> (unparsed under this scheme)", row.tag),
            }
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct PlanReportPayload {
    command: &'static str,
    subcommand: &'static str,
    root: String,
    source_version: String,
    source_label: &'static str,
    scheme: &'static str,
    mapping_source: &'static str,
    axes: AxesPayload,
    identity: String,
    semver_projection: String,
    invariant_ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    invariant_message: Option<String>,
    version_files: Vec<VersionFilePayload>,
    unrecognized_imported_version_files: Vec<String>,
    advisories: Vec<String>,
    tag_preview: Vec<migrate::TagPreviewRow>,
    manifest: crate::mvs::manifest::Manifest,
}

#[derive(Debug, Serialize)]
struct AxesPayload {
    arch: u64,
    feat: u64,
    prot: u64,
    fix: u64,
}

#[derive(Debug, Serialize)]
struct VersionFilePayload {
    path: String,
    kind: &'static str,
}

// ── backfill ──────────────────────────────────────────────────────────────

fn run_backfill(args: MigrateBackfillArgs) -> i32 {
    match try_backfill(&args) {
        Ok(report) => match render_json_or(&report, args.format, render_backfill_text) {
            Ok(()) => EXIT_SUCCESS,
            Err(error) => emit_error("migrate", args.format, error.exit_code, &error.message),
        },
        Err(error) => emit_error("migrate", args.format, error.exit_code, &error.message),
    }
}

fn try_backfill(args: &MigrateBackfillArgs) -> Result<BackfillReportPayload, CommandFailure> {
    let scheme = resolve_scheme(args.scheme.as_deref())?;

    let manifest_path = args.root.join(&args.manifest);
    let scan_policy = crate::mvs::manifest::Manifest::load(&manifest_path)
        .map(|manifest| manifest.scan_policy)
        .unwrap_or_else(|_| {
            crate::mvs::project_detect::detect_project(&args.root)
                .map(|detected| {
                    crate::mvs::project_detect::build_scan_policy(&args.root, &detected, None)
                })
                .unwrap_or_default()
        });

    let options = BackfillOptions {
        tags: if args.tags.is_empty() {
            None
        } else {
            Some(args.tags.clone())
        },
        limit: args.limit,
        scheme,
        scheme_regex: args.scheme_regex.as_deref(),
        map_spec: args.map.as_deref(),
    };

    let result = migrate::run_backfill(&args.root, &scan_policy, &options)
        .map_err(|error| CommandFailure::new(EXIT_MIGRATE_ERROR, format!("{error:#}")))?;

    Ok(BackfillReportPayload {
        command: "migrate",
        subcommand: "backfill",
        root: args.root.display().to_string(),
        resolved_scheme: result.resolved_scheme.map(|scheme| scheme.as_str()),
        tags_considered: result.tags_considered,
        skipped_tags: result
            .skipped_tags
            .into_iter()
            .map(|(tag, reason)| SkippedTagPayload { tag, reason })
            .collect(),
        transitions: result.transitions,
        violation_count: result.violation_count,
    })
}

fn render_backfill_text(report: &BackfillReportPayload) -> Result<(), CommandFailure> {
    println!(
        "migrate backfill: {} ({} tag(s) considered, scheme: {})",
        report.root,
        report.tags_considered.len(),
        report.resolved_scheme.unwrap_or("unresolved")
    );
    for skipped in &report.skipped_tags {
        println!("  skipped {}: {}", skipped.tag, skipped.reason);
    }
    for transition in &report.transitions {
        let marker = if transition.violation.is_some() {
            "!"
        } else {
            " "
        };
        println!(
            "  {marker} {} -> {} [{}]: +{}/-{} features, +{}/-{} protocols, +{}/-{} public API",
            transition.from_tag,
            transition.to_tag,
            transition.bump_level.as_str(),
            transition.features_added,
            transition.features_removed,
            transition.protocols_added,
            transition.protocols_removed,
            transition.public_api_added,
            transition.public_api_removed,
        );
        if let Some(violation) = &transition.violation {
            println!("      {violation}");
        }
    }
    println!(
        "migrate backfill: {} SemVer-honesty violation(s) across {} transition(s).",
        report.violation_count,
        report.transitions.len()
    );
    Ok(())
}

#[derive(Debug, Serialize)]
struct BackfillReportPayload {
    command: &'static str,
    subcommand: &'static str,
    root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved_scheme: Option<&'static str>,
    tags_considered: Vec<String>,
    skipped_tags: Vec<SkippedTagPayload>,
    transitions: Vec<migrate::TransitionReport>,
    violation_count: usize,
}

#[derive(Debug, Serialize)]
struct SkippedTagPayload {
    tag: String,
    reason: String,
}

// ── apply ─────────────────────────────────────────────────────────────────

fn run_apply(args: MigrateApplyArgs) -> i32 {
    match try_apply(&args) {
        Ok(report) => match render_json_or(&report, args.format, render_apply_text) {
            Ok(()) => EXIT_SUCCESS,
            Err(error) => emit_error("migrate", args.format, error.exit_code, &error.message),
        },
        Err(error) => emit_error("migrate", args.format, error.exit_code, &error.message),
    }
}

fn try_apply(args: &MigrateApplyArgs) -> Result<ApplyReportPayload, CommandFailure> {
    let common = &args.common;
    let manifest_path = common.root.join(&common.manifest);

    if manifest_path.exists() && !args.force {
        return Err(CommandFailure::new(
            EXIT_MIGRATE_ERROR,
            format!(
                "manifest already exists at `{}`. Use --force to overwrite (a snapshot will still \
                 be saved so you can `migrate rollback`).",
                manifest_path.display()
            ),
        ));
    }

    let plan = try_plan(common)?;

    let previous_manifest_contents = if manifest_path.exists() {
        Some(fs::read_to_string(&manifest_path).map_err(|error| {
            CommandFailure::new(
                EXIT_MIGRATE_ERROR,
                format!(
                    "failed to read existing manifest `{}`: {error}",
                    manifest_path.display()
                ),
            )
        })?)
    } else {
        None
    };

    let mut version_file_backups = Vec::new();
    for entry in &plan.manifest.release.version_files {
        let file_path = common.root.join(&entry.path);
        if let Ok(contents) = fs::read_to_string(&file_path) {
            version_file_backups.push(migrate::VersionFileBackup {
                path: entry.path.clone(),
                previous_contents: contents,
            });
        }
    }

    let snapshot = migrate::MigrationSnapshot {
        created_at_unix: current_unix_timestamp(),
        manifest_path: manifest_path.display().to_string(),
        previous_manifest_contents,
        version_files: version_file_backups,
    };
    snapshot.save(&common.root).map_err(|error| {
        CommandFailure::new(
            EXIT_MIGRATE_ERROR,
            format!("failed to save migration snapshot: {error:#}"),
        )
    })?;

    plan.manifest.write(&manifest_path).map_err(|error| {
        CommandFailure::new(
            EXIT_MIGRATE_ERROR,
            format!(
                "failed to write manifest to `{}`: {error:#} (a snapshot was saved; nothing else \
                 was changed)",
                manifest_path.display()
            ),
        )
    })?;

    let mut synced_files = Vec::new();
    for entry in &plan.manifest.release.version_files {
        let projected = version_sources::projected_version(entry, &plan.manifest.identity);
        match version_sources::write_version(&common.root, entry, &projected) {
            Ok(changed) => synced_files.push(SyncedFilePayload {
                path: entry.path.clone(),
                projected_version: projected,
                changed,
            }),
            Err(error) => {
                return Err(CommandFailure::new(
                    EXIT_MIGRATE_ERROR,
                    format!(
                        "manifest was written but failed to sync `{}`: {error:#}; run `migrate \
                         rollback` to undo everything",
                        entry.path
                    ),
                ));
            }
        }
    }

    Ok(ApplyReportPayload {
        command: "migrate",
        subcommand: "apply",
        manifest_path: manifest_path.display().to_string(),
        snapshot_path: migrate::MigrationSnapshot::path(&common.root)
            .display()
            .to_string(),
        identity: plan.manifest.identity.mvs.clone(),
        synced_files,
    })
}

fn render_apply_text(report: &ApplyReportPayload) -> Result<(), CommandFailure> {
    println!(
        "migrate apply: wrote `{}` (identity: {})",
        report.manifest_path, report.identity
    );
    for file in &report.synced_files {
        let status = if file.changed {
            "updated"
        } else {
            "already in sync"
        };
        println!("  {} -> {} ({status})", file.path, file.projected_version);
    }
    println!(
        "migrate apply: snapshot saved to `{}`; run `mvs-manager migrate rollback` to undo.",
        report.snapshot_path
    );
    Ok(())
}

#[derive(Debug, Serialize)]
struct ApplyReportPayload {
    command: &'static str,
    subcommand: &'static str,
    manifest_path: String,
    snapshot_path: String,
    identity: String,
    synced_files: Vec<SyncedFilePayload>,
}

#[derive(Debug, Serialize)]
struct SyncedFilePayload {
    path: String,
    projected_version: String,
    changed: bool,
}

// ── rollback ──────────────────────────────────────────────────────────────

fn run_rollback(args: MigrateRollbackArgs) -> i32 {
    match try_rollback(&args) {
        Ok(report) => match render_json_or(&report, args.format, render_rollback_text) {
            Ok(()) => EXIT_SUCCESS,
            Err(error) => emit_error("migrate", args.format, error.exit_code, &error.message),
        },
        Err(error) => emit_error("migrate", args.format, error.exit_code, &error.message),
    }
}

fn try_rollback(args: &MigrateRollbackArgs) -> Result<RollbackReportPayload, CommandFailure> {
    let snapshot = migrate::MigrationSnapshot::load(&args.root)
        .map_err(|error| CommandFailure::new(EXIT_MIGRATE_ERROR, format!("{error:#}")))?;

    let manifest_path = PathBuf::from(&snapshot.manifest_path);
    let manifest_action = match &snapshot.previous_manifest_contents {
        Some(contents) => {
            fs::write(&manifest_path, contents).map_err(|error| {
                CommandFailure::new(
                    EXIT_MIGRATE_ERROR,
                    format!("failed to restore `{}`: {error}", manifest_path.display()),
                )
            })?;
            "restored"
        }
        None => {
            if manifest_path.exists() {
                fs::remove_file(&manifest_path).map_err(|error| {
                    CommandFailure::new(
                        EXIT_MIGRATE_ERROR,
                        format!("failed to remove `{}`: {error}", manifest_path.display()),
                    )
                })?;
            }
            "removed (did not exist before apply)"
        }
    };

    let mut restored_files = Vec::new();
    for backup in &snapshot.version_files {
        let file_path = args.root.join(&backup.path);
        fs::write(&file_path, &backup.previous_contents).map_err(|error| {
            CommandFailure::new(
                EXIT_MIGRATE_ERROR,
                format!("failed to restore `{}`: {error}", file_path.display()),
            )
        })?;
        restored_files.push(backup.path.clone());
    }

    migrate::MigrationSnapshot::remove(&args.root).map_err(|error| {
        CommandFailure::new(
            EXIT_MIGRATE_ERROR,
            format!("failed to remove migration snapshot: {error:#}"),
        )
    })?;

    Ok(RollbackReportPayload {
        command: "migrate",
        subcommand: "rollback",
        manifest_path: manifest_path.display().to_string(),
        manifest_action,
        restored_files,
    })
}

fn render_rollback_text(report: &RollbackReportPayload) -> Result<(), CommandFailure> {
    println!(
        "migrate rollback: `{}` {}.",
        report.manifest_path, report.manifest_action
    );
    for path in &report.restored_files {
        println!("  restored {path}");
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct RollbackReportPayload {
    command: &'static str,
    subcommand: &'static str,
    manifest_path: String,
    manifest_action: &'static str,
    restored_files: Vec<String>,
}

// ── shared output helper ─────────────────────────────────────────────────

fn render_json_or<T: Serialize>(
    report: &T,
    format: OutputFormat,
    render_text: impl FnOnce(&T) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    match format {
        OutputFormat::Text => render_text(report),
        OutputFormat::Json => emit_json(report).map_err(|error| {
            CommandFailure::new(
                EXIT_MIGRATE_ERROR,
                format!("failed to render migrate output: {}", error.message),
            )
        }),
    }
}
