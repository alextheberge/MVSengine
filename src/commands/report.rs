// SPDX-License-Identifier: AGPL-3.0-only
use anyhow::Context;
use serde::Serialize;

use crate::cli::{OutputFormat, ReportArgs, EXIT_MANIFEST_ERROR, EXIT_REPORT_ERROR, EXIT_SUCCESS};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::mvs::manifest::Manifest;
use crate::mvs::reader::compare_manifests;

/// @mvs-feature("manifest_comparison_reporting")
/// @mvs-protocol("cli_report_command")
pub fn run(args: ReportArgs) -> i32 {
    match try_run(&args) {
        Ok(report) => match render_report(&report, args.format) {
            Ok(()) => report.exit_code,
            Err(error) => emit_error("report", args.format, error.exit_code, &error.message),
        },
        Err(error) => emit_error("report", args.format, error.exit_code, &error.message),
    }
}

fn try_run(args: &ReportArgs) -> std::result::Result<ManifestReport, CommandFailure> {
    let base = Manifest::load(&args.base_manifest)
        .with_context(|| {
            format!(
                "failed to read base manifest: {}",
                args.base_manifest.display()
            )
        })
        .map_err(|error| CommandFailure::new(EXIT_MANIFEST_ERROR, format!("{error:#}")))?;
    let target = Manifest::load(&args.target_manifest)
        .with_context(|| {
            format!(
                "failed to read target manifest: {}",
                args.target_manifest.display()
            )
        })
        .map_err(|error| CommandFailure::new(EXIT_MANIFEST_ERROR, format!("{error:#}")))?;

    let comparison = compare_manifests(&base, &target);
    let changed_sections = comparison
        .changed_sections()
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let status = if comparison.is_changed() {
        "changed"
    } else {
        "unchanged"
    };

    Ok(ManifestReport {
        command: "report",
        status,
        exit_code: EXIT_SUCCESS,
        base_manifest: args.base_manifest.display().to_string(),
        target_manifest: args.target_manifest.display().to_string(),
        change_count: comparison.change_count(),
        changed_sections,
        comparison,
    })
}

fn render_report(
    report: &ManifestReport,
    format: OutputFormat,
) -> std::result::Result<(), CommandFailure> {
    match format {
        OutputFormat::Text => {
            if report.status == "unchanged" {
                println!("Manifest report: no tracked differences.");
                return Ok(());
            }

            println!(
                "Manifest report: {} change(s) across {}.",
                report.change_count,
                report.changed_sections.join(", ")
            );

            if report.comparison.identity.arch_delta != 0
                || report.comparison.identity.feat_delta != 0
                || report.comparison.identity.prot_delta != 0
                || report.comparison.identity.context_changed
            {
                println!(
                    "- Identity: {} -> {} (ARCH {:+}, FEAT {:+}, PROT {:+})",
                    report.comparison.identity.base,
                    report.comparison.identity.target,
                    report.comparison.identity.arch_delta,
                    report.comparison.identity.feat_delta,
                    report.comparison.identity.prot_delta
                );
            }
            if report.comparison.compatibility.host_range_changed
                || report.comparison.compatibility.extension_range_changed
                || !report
                    .comparison
                    .compatibility
                    .added_legacy_shims
                    .is_empty()
                || !report
                    .comparison
                    .compatibility
                    .removed_legacy_shims
                    .is_empty()
            {
                println!(
                    "- Compatibility: host range changed={}, extension range changed={}, shim delta +{}/-{}",
                    report.comparison.compatibility.host_range_changed,
                    report.comparison.compatibility.extension_range_changed,
                    report.comparison.compatibility.added_legacy_shims.len(),
                    report.comparison.compatibility.removed_legacy_shims.len()
                );
            }
            if !report.comparison.capabilities.changes.is_empty() {
                println!(
                    "- Capabilities: {} field-level change(s).",
                    report.comparison.capabilities.changes.len()
                );
            }
            if report.comparison.ai_contract.tool_schema_version_changed
                || report.comparison.ai_contract.tool_schema_hash_changed
                || report.comparison.ai_contract.prompt_contract_id_changed
                || !report
                    .comparison
                    .ai_contract
                    .required_model_capabilities
                    .is_empty()
                || !report
                    .comparison
                    .ai_contract
                    .provided_model_capabilities
                    .is_empty()
            {
                println!(
                    "- AI contract: schema version changed={}, hash changed={}, prompt contract changed={}.",
                    report.comparison.ai_contract.tool_schema_version_changed,
                    report.comparison.ai_contract.tool_schema_hash_changed,
                    report.comparison.ai_contract.prompt_contract_id_changed
                );
            }
            if !report.comparison.environment.profiles.is_empty()
                || !report.comparison.environment.runtime_constraints.is_empty()
            {
                println!(
                    "- Environment: profile delta +{}/-{}, runtime constraint changes {}.",
                    report.comparison.environment.profiles.added.len(),
                    report.comparison.environment.profiles.removed.len(),
                    report.comparison.environment.runtime_constraints.len()
                );
            }
            if !report.comparison.scan_policy.changes.is_empty() {
                println!(
                    "- Scan policy: {} field-level change(s).",
                    report.comparison.scan_policy.changes.len()
                );
            }
            if report.comparison.evidence.feature_hash_changed
                || report.comparison.evidence.protocol_hash_changed
                || report.comparison.evidence.public_api_hash_changed
                || !report.comparison.evidence.diff.is_empty()
            {
                println!(
                    "- Evidence: feature delta +{}/-{}, protocol delta +{}/-{}, public API delta +{}/-{}.",
                    report.comparison.evidence.diff.features.added.len(),
                    report.comparison.evidence.diff.features.removed.len(),
                    report.comparison.evidence.diff.protocols.added.len(),
                    report.comparison.evidence.diff.protocols.removed.len(),
                    report.comparison.evidence.diff.public_api.added.len(),
                    report.comparison.evidence.diff.public_api.removed.len()
                );
            }

            Ok(())
        }
        OutputFormat::Json => emit_json(report).map_err(|error| {
            CommandFailure::new(
                EXIT_REPORT_ERROR,
                format!("failed to render report output: {}", error.message),
            )
        }),
    }
}

#[derive(Debug, Serialize)]
struct ManifestReport {
    command: &'static str,
    status: &'static str,
    exit_code: i32,
    base_manifest: String,
    target_manifest: String,
    change_count: usize,
    changed_sections: Vec<String>,
    comparison: crate::mvs::reader::ManifestComparison,
}
