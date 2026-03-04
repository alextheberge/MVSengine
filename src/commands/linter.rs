// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::BTreeSet;

use anyhow::{anyhow, Context, Result};

use crate::cli::LintArgs;
use crate::mvs::crawler::{crawl_codebase, ApiSignature};
use crate::mvs::hashing::{hash_file, hash_items};
use crate::mvs::manifest::Manifest;

/// @mvs-feature("manifest_linting")
/// @mvs-protocol("cli_lint_command")
pub fn run(args: LintArgs) -> Result<()> {
    let manifest = Manifest::load(&args.manifest)
        .with_context(|| format!("failed to load manifest: {}", args.manifest.display()))?;

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

    let mut failures = Vec::new();

    if manifest.evidence.feature_hash != feature_hash {
        failures.push(
            "Feature drift detected: evidence.feature_hash differs from code scan. Run `mvs-manager generate` to evaluate FEAT increment."
                .to_string(),
        );
    }

    if manifest.evidence.protocol_hash != protocol_hash {
        let example = crawl
            .protocol_occurrences
            .first()
            .map(|entry| entry.file.as_str())
            .unwrap_or("unknown source");
        failures.push(format!(
            "Protocol decorator drift detected (example source: {}). PROT increment is required.",
            example
        ));
    }

    if manifest.evidence.public_api_hash != public_api_hash {
        let example = crawl
            .public_api
            .first()
            .map(|entry| entry.file.as_str())
            .unwrap_or("unknown source");
        failures.push(format!(
            "Public API signature drift detected (example source: {}). Build must fail until PROT is incremented and manifest is regenerated.",
            example
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
        println!("Lint passed: manifest evidence matches current code and contract surfaces.");
        return Ok(());
    }

    println!("Lint failed with {} issue(s):", failures.len());
    for failure in &failures {
        println!("- {failure}");
    }

    Err(anyhow!("lint failed"))
}

fn hash_public_api(signatures: &[ApiSignature]) -> String {
    hash_items(
        signatures
            .iter()
            .map(|item| format!("{}|{}", item.file, item.signature)),
    )
}
