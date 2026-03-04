use anyhow::{Context, Result};

use crate::cli::GenerateArgs;
use crate::mvs::crawler::{crawl_codebase, ApiSignature, CrawlReport};
use crate::mvs::hashing::{hash_file, hash_items};
use crate::mvs::manifest::{LegacyShim, Manifest, ProtocolRange};

/// @mvs-feature("manifest_generation")
/// @mvs-protocol("cli_generate_command")
pub fn run(args: GenerateArgs) -> Result<()> {
    let context = args.context.as_deref().unwrap_or("cli");
    let mut manifest = Manifest::load_if_exists(&args.manifest, context)?;

    let crawl = crawl_codebase(&args.root)
        .with_context(|| format!("failed to crawl source root: {}", args.root.display()))?;

    let feature_hash = hash_items(crawl.feature_tags.iter().map(String::as_str));
    let protocol_hash = hash_items(crawl.protocol_tags.iter().map(String::as_str));
    let public_api_hash = hash_public_api(&crawl.public_api);

    let ai_schema_hash = if let Some(schema_path) = args.ai_schema.as_ref() {
        hash_file(schema_path)
            .with_context(|| format!("failed to hash AI schema file: {}", schema_path.display()))?
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
    let range_strategy = resolve_range_strategy(&args);

    manifest.identity.arch += decision.arch_increment;
    manifest.identity.feat += decision.feat_increment;
    manifest.identity.prot += decision.prot_increment;
    manifest.identity.cont = context.to_string();
    manifest.sync_identity_string();

    manifest.evidence.feature_hash = feature_hash;
    manifest.evidence.protocol_hash = protocol_hash;
    manifest.evidence.public_api_hash = public_api_hash;

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

    println!("MVS identity: {}", manifest.identity.mvs);
    if reasons_to_persist.is_empty() {
        println!("No axis increments required; evidence was refreshed.");
    } else {
        for reason in &reasons_to_persist {
            println!("- {reason}");
        }
    }

    if args.dry_run {
        println!("Dry run enabled; manifest not written.");
    } else {
        manifest.write(&args.manifest)?;
        println!("Manifest written to {}", args.manifest.display());
    }

    Ok(())
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

    use crate::cli::GenerateArgs;
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
        };

        match resolve_range_strategy(&args) {
            super::RangeStrategy::BackwardsCompatible(window) => assert_eq!(window, 2),
            _ => panic!("unexpected strategy"),
        }
    }
}
