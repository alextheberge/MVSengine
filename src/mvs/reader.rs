use crate::mvs::manifest::Manifest;

#[derive(Debug, Clone, Default)]
pub struct ValidationResult {
    pub compatible: bool,
    pub degraded: bool,
    pub reasons: Vec<String>,
}

pub fn validate_host_extension(
    host: &Manifest,
    extension: &Manifest,
    context: Option<&str>,
    allow_shims: bool,
    host_model_capabilities_override: Option<&[String]>,
) -> ValidationResult {
    let mut reasons = Vec::new();
    let mut compatible = true;
    let mut degraded = false;

    let host_prot = host.identity.prot;
    let extension_prot = extension.identity.prot;

    let host_accepts_extension = host.compatibility.extension_range.contains(extension_prot);
    let extension_accepts_host = extension.compatibility.host_range.contains(host_prot);

    if !host_accepts_extension || !extension_accepts_host {
        let shim_available = allow_shims && has_shim(host, extension_prot, host_prot);
        if shim_available {
            degraded = true;
            reasons.push(format!(
                "Protocol range mismatch resolved through legacy shim (extension PROT {}, host PROT {}).",
                extension_prot, host_prot
            ));
        } else {
            compatible = false;
            reasons.push(format!(
                "Protocol range mismatch: extension requires host {}-{}, host exposes {}-{} and is at PROT {}.",
                extension.compatibility.host_range.min_prot,
                extension.compatibility.host_range.max_prot,
                host.compatibility.extension_range.min_prot,
                host.compatibility.extension_range.max_prot,
                host_prot
            ));

            if let Some(reason) = extension.latest_protocol_reason(extension_prot) {
                reasons.push(format!(
                    "Extension protocol {} rationale: {}",
                    extension_prot, reason
                ));
            }
            if let Some(reason) = host.latest_protocol_reason(host_prot) {
                reasons.push(format!("Host protocol {} rationale: {}", host_prot, reason));
            }
        }
    }

    let target_context = context.unwrap_or(&extension.identity.cont);
    if !context_satisfies(target_context, &extension.identity.cont) {
        compatible = false;
        reasons.push(format!(
            "Context mismatch: extension CONT `{}` is not compatible with target `{}`.",
            extension.identity.cont, target_context
        ));
    }

    if !host.environment.profiles.is_empty()
        && !host
            .environment
            .profiles
            .iter()
            .any(|profile| context_pair_compatible(profile, target_context))
    {
        compatible = false;
        reasons.push(format!(
            "Host runtime profiles do not support target context `{}`.",
            target_context
        ));
    }

    for (capability, required) in &extension.capabilities {
        if *required && !host.capabilities.get(capability).copied().unwrap_or(false) {
            compatible = false;
            reasons.push(format!(
                "Missing required capability: host does not provide `{}`.",
                capability
            ));
        }
    }

    if extension.ai_contract.tool_schema_version > host.ai_contract.tool_schema_version {
        compatible = false;
        reasons.push(format!(
            "AI contract schema version unsupported: extension requires {}, host provides {}.",
            extension.ai_contract.tool_schema_version, host.ai_contract.tool_schema_version
        ));
    }

    let host_runtime_capabilities: Vec<String> = host_model_capabilities_override
        .map(|capabilities| capabilities.to_vec())
        .unwrap_or_else(|| {
            if host.ai_contract.provided_model_capabilities.is_empty() {
                host.ai_contract.required_model_capabilities.clone()
            } else {
                host.ai_contract.provided_model_capabilities.clone()
            }
        });

    let host_capability_set: std::collections::BTreeSet<String> = host_runtime_capabilities
        .iter()
        .map(|item| item.trim().to_ascii_lowercase())
        .filter(|item| !item.is_empty())
        .collect();

    let missing_ai_capabilities: Vec<String> = extension
        .ai_contract
        .required_model_capabilities
        .iter()
        .filter_map(|required| {
            let normalized = required.trim().to_ascii_lowercase();
            if host_capability_set.contains(&normalized) {
                None
            } else {
                Some(required.clone())
            }
        })
        .collect();

    if !missing_ai_capabilities.is_empty() {
        compatible = false;
        reasons.push(format!(
            "AI capability mismatch: host runtime is missing required model capabilities: {}.",
            missing_ai_capabilities.join(", ")
        ));
    }

    if compatible && reasons.is_empty() {
        reasons.push(
            "Compatible: protocol, context, capabilities, and AI contract checks all passed."
                .to_string(),
        );
    }

    ValidationResult {
        compatible,
        degraded,
        reasons,
    }
}

fn has_shim(host: &Manifest, from_prot: u64, to_prot: u64) -> bool {
    host.compatibility
        .legacy_shims
        .iter()
        .any(|shim| shim.from_prot == from_prot && shim.to_prot == to_prot)
}

fn context_satisfies(actual: &str, required: &str) -> bool {
    actual == required || actual.starts_with(&format!("{required}."))
}

fn context_pair_compatible(left: &str, right: &str) -> bool {
    context_satisfies(left, right) || context_satisfies(right, left)
}

#[cfg(test)]
mod tests {
    use super::validate_host_extension;
    use crate::mvs::manifest::{LegacyShim, Manifest, ProtocolRange};

    fn base_manifest(context: &str, prot: u64) -> Manifest {
        let mut manifest = Manifest::default_for_context(context);
        manifest.identity.prot = prot;
        manifest.sync_identity_string();
        manifest.compatibility.host_range = ProtocolRange {
            min_prot: prot,
            max_prot: prot,
        };
        manifest.compatibility.extension_range = ProtocolRange {
            min_prot: prot,
            max_prot: prot,
        };
        manifest
    }

    #[test]
    fn validates_when_ranges_and_capabilities_match() {
        let mut host = base_manifest("cli", 2);
        host.capabilities
            .insert("offline_storage".to_string(), true);

        let mut extension = base_manifest("cli", 2);
        extension
            .capabilities
            .insert("offline_storage".to_string(), true);

        let result = validate_host_extension(&host, &extension, None, true, None);
        assert!(result.compatible);
        assert!(!result.degraded);
    }

    #[test]
    fn returns_degraded_when_protocol_out_of_range_but_shim_exists() {
        let mut host = base_manifest("cli", 2);
        host.compatibility.extension_range = ProtocolRange {
            min_prot: 2,
            max_prot: 2,
        };
        host.compatibility.legacy_shims.push(LegacyShim {
            from_prot: 1,
            to_prot: 2,
            adapter: "compat_v1_to_v2".to_string(),
        });

        let mut extension = base_manifest("cli", 1);
        extension.compatibility.host_range = ProtocolRange {
            min_prot: 1,
            max_prot: 1,
        };

        let result = validate_host_extension(&host, &extension, None, true, None);
        assert!(result.compatible);
        assert!(result.degraded);
    }

    #[test]
    fn fails_when_required_capability_is_missing() {
        let host = base_manifest("cli", 1);
        let mut extension = base_manifest("cli", 1);
        extension.capabilities.insert("streaming".to_string(), true);

        let result = validate_host_extension(&host, &extension, None, true, None);
        assert!(!result.compatible);
        assert!(result
            .reasons
            .iter()
            .any(|reason| reason.contains("Missing required capability")));
    }

    #[test]
    fn context_hierarchy_allows_general_extension_on_specific_runtime() {
        let mut host = base_manifest("edge.mobile", 1);
        host.environment.profiles = vec!["edge.mobile".to_string()];
        let extension = base_manifest("edge", 1);

        let result = validate_host_extension(&host, &extension, None, true, None);
        assert!(result.compatible);
    }

    #[test]
    fn fails_when_ai_runtime_capabilities_do_not_satisfy_extension() {
        let host = base_manifest("cli", 1);
        let mut extension = base_manifest("cli", 1);
        extension.ai_contract.required_model_capabilities = vec!["reasoning-v1".to_string()];

        let result = validate_host_extension(
            &host,
            &extension,
            None,
            true,
            Some(&["tool_calling".to_string()]),
        );
        assert!(!result.compatible);
        assert!(result
            .reasons
            .iter()
            .any(|reason| reason.contains("AI capability mismatch")));
    }
}
