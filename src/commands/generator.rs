use anyhow::{Context, Result};

use crate::cli::GenerateArgs;
use crate::mvs::crawler::{crawl_codebase, ApiSignature, CrawlReport};
use crate::mvs::hashing::{hash_file, hash_items};
use crate::mvs::manifest::{Manifest, ProtocolRange};

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

    normalize_range(
        &mut manifest.compatibility.host_range,
        manifest.identity.prot,
    );
    normalize_range(
        &mut manifest.compatibility.extension_range,
        manifest.identity.prot,
    );

    println!("MVS identity: {}", manifest.identity.mvs);
    if decision.reasons.is_empty() {
        println!("No axis increments required; evidence was refreshed.");
    } else {
        for reason in &decision.reasons {
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

#[cfg(test)]
mod tests {
    use crate::mvs::crawler::{ApiSignature, CrawlReport, TagOccurrence};
    use crate::mvs::manifest::Manifest;

    use super::{derive_axis_decision, AxisInputs};

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
}
