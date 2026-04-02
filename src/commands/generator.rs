// SPDX-License-Identifier: AGPL-3.0-only
use anyhow::Context;
use serde::Serialize;

use crate::cli::{
    GenerateArgs, OutputFormat, EXIT_GENERATE_ERROR, EXIT_MANIFEST_ERROR, EXIT_SUCCESS,
};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::mvs::crawler::{crawl_codebase, ApiSignature, CrawlReport};
use crate::mvs::hashing::{hash_file, hash_items};
use crate::mvs::manifest::{InventoryDiff, LegacyShim, Manifest, ProtocolRange, PublicApiSnapshot};

/// @mvs-feature("manifest_generation")
/// @mvs-protocol("cli_generate_command")
pub fn run(args: GenerateArgs) -> i32 {
    match try_run(&args) {
        Ok(report) => match render_generate_report(&report, args.format) {
            Ok(()) => report.exit_code,
            Err(error) => emit_error("generate", args.format, error.exit_code, &error.message),
        },
        Err(error) => emit_error("generate", args.format, error.exit_code, &error.message),
    }
}

fn try_run(args: &GenerateArgs) -> std::result::Result<GenerateReport, CommandFailure> {
    let context = args.context.as_deref().unwrap_or("cli");
    let mut manifest = Manifest::load_if_exists(&args.manifest, context).map_err(|error| {
        CommandFailure::new(
            EXIT_MANIFEST_ERROR,
            format!(
                "failed to load manifest `{}`: {error:#}",
                args.manifest.display()
            ),
        )
    })?;
    let previous_identity = manifest.identity.mvs.clone();
    let previous_evidence = manifest.evidence.clone();

    apply_scan_policy_overrides(&mut manifest, args);

    let crawl = crawl_codebase(&args.root, &manifest.scan_policy)
        .with_context(|| format!("failed to crawl source root: {}", args.root.display()))
        .map_err(|error| CommandFailure::new(EXIT_GENERATE_ERROR, format!("{error:#}")))?;

    let feature_inventory: Vec<String> = crawl.feature_tags.iter().cloned().collect();
    let protocol_inventory: Vec<String> = crawl.protocol_tags.iter().cloned().collect();
    let public_api_inventory = build_public_api_inventory(&crawl.public_api);
    let inventory_diff = previous_evidence.semantic_diff(
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
            .map_err(|error| CommandFailure::new(EXIT_GENERATE_ERROR, format!("{error:#}")))?
    } else {
        manifest.ai_contract.tool_schema_hash.clone()
    };

    let decision = derive_axis_decision(AxisInputs {
        manifest: &manifest,
        crawl: &crawl,
        feature_hash: &feature_hash,
        protocol_hash: &protocol_hash,
        public_api_hash: &public_api_hash,
        ai_schema_hash: &ai_schema_hash,
        arch_break: args.arch_break,
        arch_reason: args.arch_reason.as_deref(),
    });
    let range_strategy = resolve_range_strategy(args);

    manifest.identity.arch += decision.arch_increment;
    manifest.identity.feat += decision.feat_increment;
    manifest.identity.prot += decision.prot_increment;
    manifest.identity.cont = context.to_string();
    manifest.sync_identity_string();

    manifest.evidence.feature_hash = feature_hash;
    manifest.evidence.protocol_hash = protocol_hash;
    manifest.evidence.public_api_hash = public_api_hash;
    manifest.evidence.feature_inventory = feature_inventory;
    manifest.evidence.protocol_inventory = protocol_inventory;
    manifest.evidence.public_api_inventory = public_api_inventory;

    if !ai_schema_hash.is_empty() {
        manifest.ai_contract.tool_schema_hash = ai_schema_hash;
    }

    if !manifest
        .environment
        .profiles
        .iter()
        .any(|profile| profile == context)
    {
        manifest.environment.profiles.push(context.to_string());
    }

    let mut reasons_to_persist = decision.reasons.clone();
    if let Some(strategy_reason) = apply_range_strategy(&mut manifest, range_strategy) {
        reasons_to_persist.push(strategy_reason);
    }
    manifest.append_history_entry(reasons_to_persist.clone());

    if !args.dry_run {
        manifest.write(&args.manifest).map_err(|error| {
            CommandFailure::new(
                EXIT_MANIFEST_ERROR,
                format!(
                    "failed to write manifest `{}`: {error:#}",
                    args.manifest.display()
                ),
            )
        })?;
    }

    Ok(GenerateReport {
        command: "generate",
        status: "ok",
        exit_code: EXIT_SUCCESS,
        manifest_path: args.manifest.display().to_string(),
        root: args.root.display().to_string(),
        context: context.to_string(),
        dry_run: args.dry_run,
        manifest_written: !args.dry_run,
        range_strategy: range_strategy.label().to_string(),
        scan_policy: manifest.scan_policy.clone(),
        identity: GenerateIdentityReport {
            previous: previous_identity,
            current: manifest.identity.mvs.clone(),
            arch_increment: decision.arch_increment,
            feat_increment: decision.feat_increment,
            prot_increment: decision.prot_increment,
        },
        reasons: reasons_to_persist,
        evidence: GenerateEvidenceReport {
            feature_hash: manifest.evidence.feature_hash.clone(),
            protocol_hash: manifest.evidence.protocol_hash.clone(),
            public_api_hash: manifest.evidence.public_api_hash.clone(),
            feature_inventory_count: manifest.evidence.feature_inventory.len(),
            protocol_inventory_count: manifest.evidence.protocol_inventory.len(),
            public_api_inventory_count: manifest.evidence.public_api_inventory.len(),
            diff: inventory_diff,
        },
    })
}

#[derive(Debug, Default)]
struct AxisDecision {
    arch_increment: u64,
    feat_increment: u64,
    prot_increment: u64,
    reasons: Vec<String>,
}

struct AxisInputs<'a> {
    manifest: &'a Manifest,
    crawl: &'a CrawlReport,
    feature_hash: &'a str,
    protocol_hash: &'a str,
    public_api_hash: &'a str,
    ai_schema_hash: &'a str,
    arch_break: bool,
    arch_reason: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
enum RangeStrategy {
    Normalize,
    LockStep,
    BackwardsCompatible(u64),
}

impl RangeStrategy {
    fn label(self) -> &'static str {
        match self {
            Self::Normalize => "normalize",
            Self::LockStep => "lock-step",
            Self::BackwardsCompatible(_) => "backwards-compatible",
        }
    }
}

#[derive(Debug, Serialize)]
struct GenerateReport {
    command: &'static str,
    status: &'static str,
    exit_code: i32,
    manifest_path: String,
    root: String,
    context: String,
    dry_run: bool,
    manifest_written: bool,
    range_strategy: String,
    scan_policy: crate::mvs::manifest::ScanPolicy,
    identity: GenerateIdentityReport,
    reasons: Vec<String>,
    evidence: GenerateEvidenceReport,
}

#[derive(Debug, Serialize)]
struct GenerateIdentityReport {
    previous: String,
    current: String,
    arch_increment: u64,
    feat_increment: u64,
    prot_increment: u64,
}

#[derive(Debug, Serialize)]
struct GenerateEvidenceReport {
    feature_hash: String,
    protocol_hash: String,
    public_api_hash: String,
    feature_inventory_count: usize,
    protocol_inventory_count: usize,
    public_api_inventory_count: usize,
    diff: InventoryDiff,
}

fn derive_axis_decision(inputs: AxisInputs<'_>) -> AxisDecision {
    let mut decision = AxisDecision::default();

    if inputs.manifest.evidence.feature_hash != inputs.feature_hash {
        decision.feat_increment = 1;
        let source = inputs
            .crawl
            .feature_occurrences
            .first()
            .map(|entry| entry.file.as_str())
            .unwrap_or("unknown source");
        decision.reasons.push(format!(
            "Feature incremented due to @mvs-feature set drift (example source: {}).",
            source
        ));
    }

    if inputs.manifest.evidence.protocol_hash != inputs.protocol_hash {
        decision.prot_increment = 1;
        let source = inputs
            .crawl
            .protocol_occurrences
            .first()
            .map(|entry| entry.file.as_str())
            .unwrap_or("unknown source");
        decision.reasons.push(format!(
            "Protocol incremented due to @mvs-protocol surface drift (example source: {}).",
            source
        ));
    }

    if inputs.manifest.evidence.public_api_hash != inputs.public_api_hash {
        decision.prot_increment = 1;
        let source = inputs
            .crawl
            .public_api
            .first()
            .map(|entry| entry.file.as_str())
            .unwrap_or("unknown source");
        decision.reasons.push(format!(
            "Protocol incremented due to public API signature drift (example source: {}).",
            source
        ));
    }

    if !inputs.ai_schema_hash.is_empty()
        && inputs.manifest.ai_contract.tool_schema_hash != inputs.ai_schema_hash
    {
        decision.prot_increment = 1;
        decision.reasons.push(
            "Protocol incremented due to AI tool schema hash drift (tool-calling contract changed)."
                .to_string(),
        );
    }

    if inputs.arch_break {
        decision.arch_increment = 1;
        decision.reasons.push(format!(
            "Architecture incremented due to declared data/system break: {}",
            inputs.arch_reason.unwrap_or("manual --arch-break flag")
        ));
    }

    decision
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

fn apply_scan_policy_overrides(manifest: &mut Manifest, args: &GenerateArgs) {
    if !args.exclude_paths.is_empty() {
        manifest.scan_policy.exclude_paths = normalize_policy_paths(&args.exclude_paths);
    }

    if !args.public_api_roots.is_empty() {
        manifest.scan_policy.public_api_roots = normalize_policy_paths(&args.public_api_roots);
    }

    if let Some(mode) = args.ts_export_following {
        manifest.scan_policy.ts_export_following = mode.into();
    }

    if let Some(mode) = args.go_export_following {
        manifest.scan_policy.go_export_following = mode.into();
    }

    if let Some(mode) = args.rust_export_following {
        manifest.scan_policy.rust_export_following = mode.into();
    }

    if let Some(mode) = args.ruby_export_following {
        manifest.scan_policy.ruby_export_following = mode.into();
    }

    if let Some(mode) = args.lua_export_following {
        manifest.scan_policy.lua_export_following = mode.into();
    }

    if let Some(mode) = args.python_export_following {
        manifest.scan_policy.python_export_following = mode.into();
    }

    if !args.python_module_roots.is_empty() {
        manifest.scan_policy.python_module_roots =
            normalize_policy_paths(&args.python_module_roots);
    }

    if !args.rust_workspace_members.is_empty() {
        manifest.scan_policy.rust_workspace_members =
            normalize_policy_paths(&args.rust_workspace_members);
    }

    if !args.public_api_includes.is_empty() {
        manifest.scan_policy.public_api_includes =
            normalize_policy_patterns(&args.public_api_includes);
    }

    if !args.public_api_excludes.is_empty() {
        manifest.scan_policy.public_api_excludes =
            normalize_policy_patterns(&args.public_api_excludes);
    }
}

fn normalize_policy_paths(paths: &[std::path::PathBuf]) -> Vec<String> {
    let mut normalized: Vec<String> = paths
        .iter()
        .filter_map(|path| {
            let value = path.to_string_lossy().replace('\\', "/");
            let value = value.trim_start_matches("./").trim_matches('/').to_string();
            if value.is_empty() {
                None
            } else {
                Some(value)
            }
        })
        .collect();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn normalize_policy_patterns(patterns: &[String]) -> Vec<String> {
    let mut normalized: Vec<String> = patterns
        .iter()
        .map(|pattern| pattern.trim().to_string())
        .filter(|pattern| !pattern.is_empty())
        .collect();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn render_generate_report(
    report: &GenerateReport,
    format: OutputFormat,
) -> std::result::Result<(), CommandFailure> {
    match format {
        OutputFormat::Text => {
            println!("MVS identity: {}", report.identity.current);
            if report.reasons.is_empty() {
                println!("No axis increments required; evidence was refreshed.");
            } else {
                for reason in &report.reasons {
                    println!("- {reason}");
                }
            }

            render_inventory_diff(&report.evidence.diff);
            println!(
                "Semantic evidence snapshots: {} features, {} protocols, {} public API signatures.",
                report.evidence.feature_inventory_count,
                report.evidence.protocol_inventory_count,
                report.evidence.public_api_inventory_count
            );
            render_scan_policy(&report.scan_policy);

            if report.manifest_written {
                println!("Manifest written to {}", report.manifest_path);
            } else {
                println!("Dry run enabled; manifest not written.");
            }

            Ok(())
        }
        OutputFormat::Json => emit_json(report),
    }
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

fn render_inventory_diff(diff: &InventoryDiff) {
    if diff.is_empty() {
        return;
    }

    if !diff.features.added.is_empty() {
        println!("- Feature tags added: {}", diff.features.added.join(", "));
    }
    if !diff.features.removed.is_empty() {
        println!(
            "- Feature tags removed: {}",
            diff.features.removed.join(", ")
        );
    }
    if !diff.protocols.added.is_empty() {
        println!("- Protocol tags added: {}", diff.protocols.added.join(", "));
    }
    if !diff.protocols.removed.is_empty() {
        println!(
            "- Protocol tags removed: {}",
            diff.protocols.removed.join(", ")
        );
    }
    if !diff.public_api.added.is_empty() {
        println!(
            "- Public API signatures added: {}",
            diff.public_api
                .added
                .iter()
                .map(|item| format!("{}|{}", item.file, item.signature))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if !diff.public_api.removed.is_empty() {
        println!(
            "- Public API signatures removed: {}",
            diff.public_api
                .removed
                .iter()
                .map(|item| format!("{}|{}", item.file, item.signature))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}

fn normalize_range(range: &mut ProtocolRange, current_prot: u64) {
    if range.min_prot > range.max_prot {
        std::mem::swap(&mut range.min_prot, &mut range.max_prot);
    }

    if current_prot < range.min_prot {
        range.min_prot = current_prot;
    }

    if current_prot > range.max_prot {
        range.max_prot = current_prot;
    }
}

fn resolve_range_strategy(args: &GenerateArgs) -> RangeStrategy {
    if args.lock_step {
        return RangeStrategy::LockStep;
    }

    if let Some(window) = args.backwards_compatible {
        return RangeStrategy::BackwardsCompatible(window);
    }

    RangeStrategy::Normalize
}

fn apply_range_strategy(manifest: &mut Manifest, strategy: RangeStrategy) -> Option<String> {
    let current_prot = manifest.identity.prot;

    match strategy {
        RangeStrategy::Normalize => {
            normalize_range(&mut manifest.compatibility.host_range, current_prot);
            normalize_range(&mut manifest.compatibility.extension_range, current_prot);
            None
        }
        RangeStrategy::LockStep => {
            manifest.compatibility.host_range = ProtocolRange {
                min_prot: current_prot,
                max_prot: current_prot,
            };
            manifest.compatibility.extension_range = ProtocolRange {
                min_prot: current_prot,
                max_prot: current_prot,
            };
            clear_auto_shims(manifest);
            Some(format!(
                "Range strategy `lock-step` applied (host/extension ranges pinned to PROT {}).",
                current_prot
            ))
        }
        RangeStrategy::BackwardsCompatible(window) => {
            let min_prot = current_prot.saturating_sub(window);
            manifest.compatibility.host_range = ProtocolRange {
                min_prot,
                max_prot: current_prot,
            };
            manifest.compatibility.extension_range = ProtocolRange {
                min_prot,
                max_prot: current_prot,
            };
            generate_auto_shims(manifest, min_prot, current_prot);
            Some(format!(
                "Range strategy `backwards-compatible-{}` applied (PROT {}-{} with generated legacy shims).",
                window, min_prot, current_prot
            ))
        }
    }
}

const AUTO_SHIM_PREFIX: &str = "auto_backward_compat";

fn clear_auto_shims(manifest: &mut Manifest) {
    manifest
        .compatibility
        .legacy_shims
        .retain(|shim| !shim.adapter.starts_with(AUTO_SHIM_PREFIX));
}

fn generate_auto_shims(manifest: &mut Manifest, min_prot: u64, current_prot: u64) {
    clear_auto_shims(manifest);

    for from_prot in min_prot..current_prot {
        manifest.compatibility.legacy_shims.push(LegacyShim {
            from_prot,
            to_prot: current_prot,
            adapter: format!("{AUTO_SHIM_PREFIX}_v{from_prot}_to_v{current_prot}"),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::cli::{GenerateArgs, OutputFormat};
    use crate::mvs::crawler::{ApiSignature, CrawlReport, TagOccurrence};
    use crate::mvs::manifest::{LegacyShim, Manifest};

    use super::{apply_range_strategy, derive_axis_decision, resolve_range_strategy, AxisInputs};

    #[test]
    fn increments_arch_and_prot_for_double_channel_break() {
        let mut manifest = Manifest::default_for_context("cli");
        manifest.evidence.feature_hash = "same-feature".to_string();
        manifest.evidence.protocol_hash = "same-protocol".to_string();
        manifest.evidence.public_api_hash = "old-api".to_string();
        manifest.ai_contract.tool_schema_hash = "same-ai".to_string();

        let crawl = CrawlReport {
            feature_tags: Default::default(),
            protocol_tags: Default::default(),
            feature_occurrences: vec![TagOccurrence {
                name: "manifest_generation".to_string(),
                file: "src/commands/generator.rs".to_string(),
            }],
            protocol_occurrences: vec![TagOccurrence {
                name: "cli_generate_command".to_string(),
                file: "src/commands/generator.rs".to_string(),
            }],
            public_api: vec![ApiSignature {
                file: "src/cli.rs".to_string(),
                signature: "ts/js:function login(username:string)".to_string(),
            }],
            public_api_boundary_decisions: Vec::new(),
            excluded_paths: Vec::new(),
        };

        let decision = derive_axis_decision(AxisInputs {
            manifest: &manifest,
            crawl: &crawl,
            feature_hash: "same-feature",
            protocol_hash: "same-protocol",
            public_api_hash: "new-api",
            ai_schema_hash: "same-ai",
            arch_break: true,
            arch_reason: Some("schema migration changed persistence layout"),
        });

        assert_eq!(decision.arch_increment, 1);
        assert_eq!(decision.prot_increment, 1);
        assert_eq!(decision.feat_increment, 0);
        assert!(decision
            .reasons
            .iter()
            .any(|reason| reason.contains("Architecture incremented")));
        assert!(decision
            .reasons
            .iter()
            .any(|reason| reason.contains("public API signature drift")));
    }

    #[test]
    fn backwards_compatible_strategy_generates_shims_and_ranges() {
        let mut manifest = Manifest::default_for_context("cli");
        manifest.identity.prot = 6;
        manifest.sync_identity_string();

        let reason =
            apply_range_strategy(&mut manifest, super::RangeStrategy::BackwardsCompatible(3));

        assert_eq!(manifest.compatibility.host_range.min_prot, 3);
        assert_eq!(manifest.compatibility.host_range.max_prot, 6);
        assert_eq!(manifest.compatibility.legacy_shims.len(), 3);
        assert!(reason
            .as_deref()
            .unwrap_or_default()
            .contains("backwards-compatible-3"));
    }

    #[test]
    fn lock_step_strategy_pins_ranges_and_clears_auto_shims() {
        let mut manifest = Manifest::default_for_context("cli");
        manifest.identity.prot = 4;
        manifest.sync_identity_string();
        manifest.compatibility.legacy_shims.push(LegacyShim {
            from_prot: 1,
            to_prot: 4,
            adapter: "auto_backward_compat_v1_to_v4".to_string(),
        });
        manifest.compatibility.legacy_shims.push(LegacyShim {
            from_prot: 2,
            to_prot: 4,
            adapter: "manual_custom_shim".to_string(),
        });

        let reason = apply_range_strategy(&mut manifest, super::RangeStrategy::LockStep);

        assert_eq!(manifest.compatibility.host_range.min_prot, 4);
        assert_eq!(manifest.compatibility.host_range.max_prot, 4);
        assert_eq!(manifest.compatibility.extension_range.min_prot, 4);
        assert_eq!(manifest.compatibility.extension_range.max_prot, 4);
        assert_eq!(manifest.compatibility.legacy_shims.len(), 1);
        assert_eq!(
            manifest.compatibility.legacy_shims[0].adapter,
            "manual_custom_shim"
        );
        assert!(reason.as_deref().unwrap_or_default().contains("lock-step"));
    }

    #[test]
    fn resolve_strategy_from_args() {
        let args = GenerateArgs {
            root: PathBuf::from("."),
            manifest: PathBuf::from("mvs.json"),
            context: None,
            ai_schema: None,
            arch_break: false,
            arch_reason: None,
            lock_step: false,
            backwards_compatible: Some(2),
            dry_run: false,
            exclude_paths: Vec::new(),
            public_api_roots: Vec::new(),
            ts_export_following: None,
            go_export_following: None,
            rust_export_following: None,
            ruby_export_following: None,
            lua_export_following: None,
            python_export_following: None,
            python_module_roots: Vec::new(),
            rust_workspace_members: Vec::new(),
            public_api_includes: Vec::new(),
            public_api_excludes: Vec::new(),
            format: OutputFormat::Text,
        };

        match resolve_range_strategy(&args) {
            super::RangeStrategy::BackwardsCompatible(window) => assert_eq!(window, 2),
            _ => panic!("unexpected strategy"),
        }
    }
}
