// SPDX-License-Identifier: AGPL-3.0-only
use serde::Serialize;

use crate::cli::{CheckManifestArgs, OutputFormat, EXIT_MANIFEST_ERROR, EXIT_SUCCESS};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::mvs::manifest::Manifest;

/// @mvs-feature("manifest_self_validation")
/// @mvs-protocol("cli_check_manifest_command")
pub fn run(args: CheckManifestArgs) -> i32 {
    let format = args.format;
    match try_run(&args) {
        Ok(report) => match render_report(&report, format) {
            Ok(()) => report.exit_code,
            Err(error) => emit_error("check-manifest", format, error.exit_code, &error.message),
        },
        Err(error) => emit_error("check-manifest", format, error.exit_code, &error.message),
    }
}

fn try_run(args: &CheckManifestArgs) -> Result<CheckManifestReport, CommandFailure> {
    let manifest = Manifest::load(&args.manifest).map_err(|error| {
        CommandFailure::new(
            EXIT_MANIFEST_ERROR,
            format!(
                "failed to load manifest `{}`: {error:#}",
                args.manifest.display()
            ),
        )
    })?;

    let mut issues: Vec<Issue> = Vec::new();

    // 1. Schema field
    if manifest.schema != "https://mvs.dev/schema/v1" {
        issues.push(Issue::warning(
            "$schema",
            format!(
                "$schema is `{}`, expected `https://mvs.dev/schema/v1`.",
                manifest.schema
            ),
        ));
    }

    // 2. Identity string consistency (manifest.validate() already checks this,
    //    but we surface it as a structured issue rather than a hard error above)
    let expected_mvs = format!(
        "{}.{}.{}-{}",
        manifest.identity.arch,
        manifest.identity.feat,
        manifest.identity.prot,
        manifest.identity.cont
    );
    if manifest.identity.mvs != expected_mvs {
        issues.push(Issue::error(
            "identity.mvs",
            format!(
                "identity.mvs is `{}` but should be `{expected_mvs}` based on arch/feat/prot/cont.",
                manifest.identity.mvs
            ),
        ));
    }

    // 3. Protocol ranges internally consistent (min <= max)
    let hr = &manifest.compatibility.host_range;
    if hr.min_prot > hr.max_prot {
        issues.push(Issue::error(
            "compatibility.host_range",
            format!(
                "host_range.min_prot ({}) > host_range.max_prot ({}) — range is inverted.",
                hr.min_prot, hr.max_prot
            ),
        ));
    }
    let er = &manifest.compatibility.extension_range;
    if er.min_prot > er.max_prot {
        issues.push(Issue::error(
            "compatibility.extension_range",
            format!(
                "extension_range.min_prot ({}) > extension_range.max_prot ({}) — range is inverted.",
                er.min_prot, er.max_prot
            ),
        ));
    }

    // 4. Legacy shim consistency: from_prot != to_prot
    for shim in &manifest.compatibility.legacy_shims {
        if shim.from_prot == shim.to_prot {
            issues.push(Issue::warning(
                "compatibility.legacy_shims",
                format!(
                    "Shim `{}` has from_prot == to_prot ({}). This is a no-op shim.",
                    shim.adapter, shim.from_prot
                ),
            ));
        }
        if shim.adapter.trim().is_empty() {
            issues.push(Issue::error(
                "compatibility.legacy_shims",
                format!(
                    "Shim from PROT {} to {} has an empty adapter identifier.",
                    shim.from_prot, shim.to_prot
                ),
            ));
        }
    }

    // 5. public_api_roots point to real files (relative to root)
    for root_pattern in &manifest.scan_policy.public_api_roots {
        // Only check patterns that look like plain file paths (no wildcards)
        if !root_pattern.contains('*') && !root_pattern.contains('?') {
            let candidate = args.root.join(root_pattern);
            if !candidate.exists() {
                issues.push(Issue::warning(
                    "scan_policy.public_api_roots",
                    format!(
                        "public_api_root `{root_pattern}` does not exist at `{}`.",
                        candidate.display()
                    ),
                ));
            }
        }
    }

    // 6. Evidence hashes look like valid hex strings (non-empty when inventory is non-empty)
    if !manifest.evidence.feature_inventory.is_empty() && manifest.evidence.feature_hash.is_empty()
    {
        issues.push(Issue::warning(
            "evidence.feature_hash",
            "feature_inventory is non-empty but feature_hash is empty. Run `generate` to refresh."
                .to_string(),
        ));
    }
    if !manifest.evidence.protocol_inventory.is_empty()
        && manifest.evidence.protocol_hash.is_empty()
    {
        issues.push(Issue::warning(
            "evidence.protocol_hash",
            "protocol_inventory is non-empty but protocol_hash is empty. Run `generate` to refresh."
                .to_string(),
        ));
    }
    if !manifest.evidence.public_api_inventory.is_empty()
        && manifest.evidence.public_api_hash.is_empty()
    {
        issues.push(Issue::warning(
            "evidence.public_api_hash",
            "public_api_inventory is non-empty but public_api_hash is empty. Run `generate` to refresh."
                .to_string(),
        ));
    }

    // 7. cont is non-empty (validate() enforces this, but surface it clearly)
    if manifest.identity.cont.trim().is_empty() {
        issues.push(Issue::error(
            "identity.cont",
            "identity.cont must not be empty.".to_string(),
        ));
    }

    let error_count = issues.iter().filter(|i| i.severity == "error").count();
    let warning_count = issues.iter().filter(|i| i.severity == "warning").count();
    let exit_code = if error_count > 0 {
        EXIT_MANIFEST_ERROR
    } else {
        EXIT_SUCCESS
    };

    Ok(CheckManifestReport {
        exit_code,
        manifest_path: args.manifest.display().to_string(),
        version: manifest.identity.mvs.clone(),
        error_count,
        warning_count,
        issues,
    })
}

fn render_report(
    report: &CheckManifestReport,
    format: OutputFormat,
) -> Result<(), CommandFailure> {
    match format {
        OutputFormat::Text => {
            if report.issues.is_empty() {
                println!(
                    "Manifest OK: {} — no issues found.",
                    report.manifest_path
                );
            } else {
                println!(
                    "Manifest check for {} ({}) — {} error(s), {} warning(s):",
                    report.manifest_path,
                    report.version,
                    report.error_count,
                    report.warning_count
                );
                for issue in &report.issues {
                    println!("  [{}] {}: {}", issue.severity.to_uppercase(), issue.field, issue.message);
                }
            }
            Ok(())
        }
        OutputFormat::Json => emit_json(report),
    }
}

#[derive(Debug, Serialize)]
struct CheckManifestReport {
    exit_code: i32,
    manifest_path: String,
    version: String,
    error_count: usize,
    warning_count: usize,
    issues: Vec<Issue>,
}

#[derive(Debug, Serialize)]
struct Issue {
    severity: &'static str,
    field: &'static str,
    message: String,
}

impl Issue {
    fn error(field: &'static str, message: String) -> Self {
        Self { severity: "error", field, message }
    }
    fn warning(field: &'static str, message: String) -> Self {
        Self { severity: "warning", field, message }
    }
}
