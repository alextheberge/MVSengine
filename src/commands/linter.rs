// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::BTreeSet;

use anyhow::Context;
use serde::Serialize;

use crate::cli::{
    GenerateArgs, LintArgs, OutputFormat, EXIT_LINT_ERROR, EXIT_LINT_FAILED, EXIT_MANIFEST_ERROR,
    EXIT_SUCCESS,
};
use crate::commands::boundary_debug::{build_boundary_debug, BoundaryDebugReport};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::mvs::crawler::{crawl_codebase, ApiSignature};
use crate::mvs::hashing::{hash_file, hash_items};
use crate::mvs::manifest::{InventoryDiff, Manifest, PublicApiSnapshot};

/// @mvs-feature("manifest_linting")
/// @mvs-protocol("cli_lint_command")
pub fn run(args: LintArgs) -> i32 {
    let explain = args.explain;
    let remediate = args.remediate;
    let format = args.format;
    match try_run(&args) {
        Ok(report) => {
            let exit_code = report.exit_code;
            match render_lint_report(&report, format, explain) {
                Ok(()) => {}
                Err(error) => return emit_error("lint", format, error.exit_code, &error.message),
            }
            if exit_code != EXIT_SUCCESS && remediate {
                return run_remediate(&args);
            }
            exit_code
        }
        Err(error) => emit_error("lint", format, error.exit_code, &error.message),
    }
}

fn run_remediate(args: &LintArgs) -> i32 {
    println!("\n-- Remediating: running `generate` to update manifest evidence --");
    let generate_args = GenerateArgs {
        root: args.root.clone(),
        manifest: args.manifest.clone(),
        context: None,
        ai_schema: args.ai_schema.clone(),
        arch_break: false,
        arch_reason: None,
        lock_step: false,
        backwards_compatible: None,
        dry_run: false,
        exclude_paths: vec![],
        public_api_roots: vec![],
        ts_export_following: None,
        go_export_following: None,
        rust_export_following: None,
        ruby_export_following: None,
        lua_export_following: None,
        python_module_roots: vec![],
        rust_workspace_members: vec![],
        python_export_following: None,
        public_api_includes: vec![],
        public_api_excludes: vec![],
        format: args.format,
    };
    let gen_exit = crate::commands::generator::run(generate_args);
    if gen_exit != EXIT_SUCCESS {
        return gen_exit;
    }
    println!("\n-- Re-linting after remediation --");
    // Re-run lint; don't pass --remediate again to avoid infinite loop
    let re_lint_args = LintArgs {
        remediate: false,
        ..args.clone()
    };
    run(re_lint_args)
}

fn try_run(args: &LintArgs) -> std::result::Result<LintReport, CommandFailure> {
    let manifest = Manifest::load(&args.manifest)
        .with_context(|| format!("failed to load manifest: {}", args.manifest.display()))
        .map_err(|error| CommandFailure::new(EXIT_MANIFEST_ERROR, format!("{error:#}")))?;

    let crawl = crawl_codebase(&args.root, &manifest.scan_policy)
        .with_context(|| format!("failed to crawl source root: {}", args.root.display()))
        .map_err(|error| CommandFailure::new(EXIT_LINT_ERROR, format!("{error:#}")))?;

    let feature_inventory: Vec<String> = crawl.feature_tags.iter().cloned().collect();
    let protocol_inventory: Vec<String> = crawl.protocol_tags.iter().cloned().collect();
    let public_api_inventory = build_public_api_inventory(&crawl.public_api);
    let inventory_diff = manifest.evidence.semantic_diff(
        &feature_inventory,
        &protocol_inventory,
        &public_api_inventory,
    );

    let feature_hash = hash_items(crawl.feature_tags.iter().map(String::as_str));
    let protocol_hash = hash_items(crawl.protocol_tags.iter().map(String::as_str));
    let public_api_hash = hash_public_api(&crawl.public_api);

    let ai_schema_hash = if let Some(schema_path) = args.ai_schema.as_ref() {
        hash_file(schema_path)
            .with_context(|| format!("failed to hash AI schema file: {}", schema_path.display()))
            .map_err(|error| CommandFailure::new(EXIT_LINT_ERROR, format!("{error:#}")))?
    } else {
        manifest.ai_contract.tool_schema_hash.clone()
    };

    let mut failures = Vec::new();
    let boundary_debug = build_boundary_debug(
        &manifest.scan_policy,
        &crawl.public_api_boundary_decisions,
        &crawl.excluded_paths,
    );

    if manifest.evidence.feature_hash != feature_hash || !inventory_diff.features.is_empty() {
        failures.push(format!(
            "Feature inventory drift detected. {} Run `mvs-manager generate` to evaluate FEAT increment.",
            summarize_string_diff(&inventory_diff.features)
        ));
    }

    if manifest.evidence.protocol_hash != protocol_hash || !inventory_diff.protocols.is_empty() {
        failures.push(format!(
            "Protocol surface drift detected. {} PROT increment is required.",
            summarize_string_diff(&inventory_diff.protocols)
        ));
    }

    if manifest.evidence.public_api_hash != public_api_hash || !inventory_diff.public_api.is_empty()
    {
        failures.push(format!(
            "Public API signature drift detected. {} Build must fail until PROT is incremented and manifest is regenerated.",
            summarize_public_api_diff(&inventory_diff.public_api)
        ));
    }

    if manifest.ai_contract.tool_schema_hash != ai_schema_hash {
        failures.push(
            "AI tool-calling schema hash drift detected. PROT increment is required for AI contract changes."
                .to_string(),
        );
    }

    if !args.available_model_capabilities.is_empty() {
        let available: BTreeSet<String> = args
            .available_model_capabilities
            .iter()
            .map(|item| item.trim().to_ascii_lowercase())
            .filter(|item| !item.is_empty())
            .collect();

        let missing: Vec<String> = manifest
            .ai_contract
            .required_model_capabilities
            .iter()
            .filter_map(|required| {
                let normalized = required.trim().to_ascii_lowercase();
                if available.contains(&normalized) {
                    None
                } else {
                    Some(required.clone())
                }
            })
            .collect();

        if !missing.is_empty() {
            failures.push(format!(
                "AI capability liveness failed: runtime is missing required model capabilities: {}.",
                missing.join(", ")
            ));
        }
    }

    if failures.is_empty() {
        return Ok(LintReport {
            command: "lint",
            status: "passed",
            exit_code: EXIT_SUCCESS,
            manifest_path: args.manifest.display().to_string(),
            root: args.root.display().to_string(),
            scan_policy: manifest.scan_policy.clone(),
            failure_count: 0,
            failures,
            boundary_debug,
            evidence: LintEvidenceReport {
                feature_hash,
                protocol_hash,
                public_api_hash,
                feature_inventory_count: feature_inventory.len(),
                protocol_inventory_count: protocol_inventory.len(),
                public_api_inventory_count: public_api_inventory.len(),
                diff: inventory_diff,
            },
        });
    }

    Ok(LintReport {
        command: "lint",
        status: "failed",
        exit_code: EXIT_LINT_FAILED,
        manifest_path: args.manifest.display().to_string(),
        root: args.root.display().to_string(),
        scan_policy: manifest.scan_policy.clone(),
        failure_count: failures.len(),
        failures,
        boundary_debug,
        evidence: LintEvidenceReport {
            feature_hash,
            protocol_hash,
            public_api_hash,
            feature_inventory_count: feature_inventory.len(),
            protocol_inventory_count: protocol_inventory.len(),
            public_api_inventory_count: public_api_inventory.len(),
            diff: inventory_diff,
        },
    })
}

fn hash_public_api(signatures: &[ApiSignature]) -> String {
    hash_items(
        signatures
            .iter()
            .map(|item| format!("{}|{}", item.file, item.signature)),
    )
}

fn build_public_api_inventory(signatures: &[ApiSignature]) -> Vec<PublicApiSnapshot> {
    let mut inventory: Vec<PublicApiSnapshot> = signatures
        .iter()
        .map(|item| PublicApiSnapshot {
            file: item.file.clone(),
            signature: item.signature.clone(),
        })
        .collect();
    inventory.sort();
    inventory.dedup();
    inventory
}

fn summarize_string_diff(diff: &crate::mvs::manifest::StringInventoryDiff) -> String {
    let mut details = Vec::new();
    if !diff.added.is_empty() {
        details.push(format!("Added: {}", diff.added.join(", ")));
    }
    if !diff.removed.is_empty() {
        details.push(format!("Removed: {}", diff.removed.join(", ")));
    }
    if details.is_empty() {
        "Semantic snapshots are out of date.".to_string()
    } else {
        details.join(" ")
    }
}

fn summarize_public_api_diff(diff: &crate::mvs::manifest::PublicApiInventoryDiff) -> String {
    let mut details = Vec::new();
    if !diff.added.is_empty() {
        details.push(format!(
            "Added: {}",
            diff.added
                .iter()
                .map(|item| format!("{}|{}", item.file, item.signature))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !diff.removed.is_empty() {
        details.push(format!(
            "Removed: {}",
            diff.removed
                .iter()
                .map(|item| format!("{}|{}", item.file, item.signature))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if details.is_empty() {
        "Semantic snapshots are out of date.".to_string()
    } else {
        details.join(" ")
    }
}

fn render_lint_report(
    report: &LintReport,
    format: OutputFormat,
    explain: bool,
) -> std::result::Result<(), CommandFailure> {
    match format {
        OutputFormat::Text => {
            if report.exit_code == EXIT_SUCCESS {
                println!(
                    "Lint passed: manifest evidence matches current code and contract surfaces."
                );
            } else {
                println!("Lint failed with {} issue(s):", report.failure_count);
                for failure in &report.failures {
                    println!("- {failure}");
                }
            }
            if let Some(boundary_debug) = report.boundary_debug.as_ref() {
                println!(
                    "- Boundary debug: {} included, {} excluded candidate declaration(s), {} excluded path(s). Use `--format json` for rule-level decisions.",
                    boundary_debug.included_count,
                    boundary_debug.excluded_count,
                    boundary_debug.excluded_path_count
                );
            }
            render_scan_policy(&report.scan_policy);
            if explain && report.exit_code != EXIT_SUCCESS {
                render_explain(&report.evidence);
            }
            emit_github_annotations(report);
            Ok(())
        }
        OutputFormat::Json => emit_json(report),
    }
}

fn render_explain(evidence: &LintEvidenceReport) {
    let diff = &evidence.diff;
    let has_feature_drift = !diff.features.added.is_empty() || !diff.features.removed.is_empty();
    let has_protocol_drift = !diff.protocols.added.is_empty() || !diff.protocols.removed.is_empty();
    let has_api_drift = !diff.public_api.added.is_empty() || !diff.public_api.removed.is_empty();

    println!("\n--- Explanation & Remediation ---");

    if has_feature_drift {
        println!("\n[Feature drift]");
        for tag in &diff.features.added {
            println!("  + added @mvs-feature: {tag}");
        }
        for tag in &diff.features.removed {
            println!("  - removed @mvs-feature: {tag}");
        }
        println!("  → A FEAT increment may be warranted. Run: mvs-manager generate");
    }

    if has_protocol_drift {
        println!("\n[Protocol surface drift]");
        for tag in &diff.protocols.added {
            println!("  + added @mvs-protocol: {tag}");
        }
        for tag in &diff.protocols.removed {
            println!("  - removed @mvs-protocol: {tag}");
        }
        println!("  → A PROT increment is required. Run: mvs-manager generate");
    }

    if has_api_drift {
        println!("\n[Public API signature drift]");
        for item in &diff.public_api.added {
            println!("  + {}  ({})", item.signature, item.file);
        }
        for item in &diff.public_api.removed {
            println!("  - {}  ({})", item.signature, item.file);
        }
        println!("  → A PROT increment is required. Run: mvs-manager generate");
        println!("  → If this change is intentional, update host_range / extension_range in mvs.json to reflect the new protocol.");
    }

    if !has_feature_drift && !has_protocol_drift && !has_api_drift {
        println!("\nEvidence hashes are stale but inventories match.");
        println!("  → Run: mvs-manager generate  (this will refresh the hashes without changing the version)");
    }

    println!();
}

/// Emit GitHub Actions workflow commands when running inside a GitHub Actions
/// environment (`GITHUB_ACTIONS=true`).  Failures become `::error::` lines
/// that appear as inline annotations on the PR diff.
fn emit_github_annotations(report: &LintReport) {
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true") {
        return;
    }
    if report.exit_code == EXIT_SUCCESS {
        println!(
            "::notice title=MVS Lint::Manifest evidence is up to date for {}",
            report.manifest_path
        );
        return;
    }
    for failure in &report.failures {
        // Emit a workflow error annotation.  We don't have a precise file/line
        // for every failure type, so we emit a title-level annotation.
        println!("::error title=MVS Lint::{failure}");
    }
    println!(
        "::error title=MVS Lint::Run `mvs-manager generate` to update evidence hashes, then re-commit."
    );
}

fn render_scan_policy(scan_policy: &crate::mvs::manifest::ScanPolicy) {
    if !scan_policy.public_api_roots.is_empty() {
        println!(
            "- Public API roots: {}",
            scan_policy.public_api_roots.join(", ")
        );
    }
    if !scan_policy.public_api_includes.is_empty() {
        println!(
            "- Public API includes: {}",
            scan_policy.public_api_includes.join(", ")
        );
    }
    if !scan_policy.ts_export_following.is_default() {
        println!(
            "- TS/JS export following: {}",
            scan_policy.ts_export_following.as_str()
        );
    }
    if !scan_policy.go_export_following.is_default() {
        println!(
            "- Go export following: {}",
            scan_policy.go_export_following.as_str()
        );
    }
    if !scan_policy.rust_export_following.is_default() {
        println!(
            "- Rust export following: {}",
            scan_policy.rust_export_following.as_str()
        );
    }
    if !scan_policy.ruby_export_following.is_default() {
        println!(
            "- Ruby export following: {}",
            scan_policy.ruby_export_following.as_str()
        );
    }
    if !scan_policy.lua_export_following.is_default() {
        println!(
            "- Lua export following: {}",
            scan_policy.lua_export_following.as_str()
        );
    }
    if !scan_policy.python_export_following.is_default() {
        println!(
            "- Python export following: {}",
            scan_policy.python_export_following.as_str()
        );
    }
    if !scan_policy.python_module_roots.is_empty() {
        println!(
            "- Python module roots: {}",
            scan_policy.python_module_roots.join(", ")
        );
    }
    if !scan_policy.rust_workspace_members.is_empty() {
        println!(
            "- Rust workspace members: {}",
            scan_policy.rust_workspace_members.join(", ")
        );
    }
    if !scan_policy.public_api_excludes.is_empty() {
        println!(
            "- Public API excludes: {}",
            scan_policy.public_api_excludes.join(", ")
        );
    }
    if !scan_policy.exclude_paths.is_empty() {
        println!(
            "- Excluded scan paths: {}",
            scan_policy.exclude_paths.join(", ")
        );
    }
}

#[derive(Debug, Serialize)]
struct LintReport {
    command: &'static str,
    status: &'static str,
    exit_code: i32,
    manifest_path: String,
    root: String,
    scan_policy: crate::mvs::manifest::ScanPolicy,
    failure_count: usize,
    failures: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    boundary_debug: Option<BoundaryDebugReport>,
    evidence: LintEvidenceReport,
}

#[derive(Debug, Serialize)]
struct LintEvidenceReport {
    feature_hash: String,
    protocol_hash: String,
    public_api_hash: String,
    feature_inventory_count: usize,
    protocol_inventory_count: usize,
    public_api_inventory_count: usize,
    diff: InventoryDiff,
}
