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
        }
    }

    match context {
        Some(target_context) => {
            if extension.identity.cont != target_context {
                compatible = false;
                reasons.push(format!(
                    "Context mismatch: extension CONT is `{}`, requested `{}`.",
                    extension.identity.cont, target_context
                ));
            }

            if !host.environment.profiles.is_empty()
                && !host
                    .environment
                    .profiles
                    .iter()
                    .any(|profile| profile == target_context)
            {
                compatible = false;
                reasons.push(format!(
                    "Host runtime profiles do not include requested context `{}`.",
                    target_context
                ));
            }
        }
        None => {
            if !host.environment.profiles.is_empty()
                && !host
                    .environment
                    .profiles
                    .iter()
                    .any(|profile| profile == &extension.identity.cont)
            {
                compatible = false;
                reasons.push(format!(
                    "Host runtime profiles do not include extension context `{}`.",
                    extension.identity.cont
                ));
            }
        }
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

        let result = validate_host_extension(&host, &extension, None, true);
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

        let result = validate_host_extension(&host, &extension, None, true);
        assert!(result.compatible);
        assert!(result.degraded);
    }

    #[test]
    fn fails_when_required_capability_is_missing() {
        let host = base_manifest("cli", 1);
        let mut extension = base_manifest("cli", 1);
        extension.capabilities.insert("streaming".to_string(), true);

        let result = validate_host_extension(&host, &extension, None, true);
        assert!(!result.compatible);
        assert!(result
            .reasons
            .iter()
            .any(|reason| reason.contains("Missing required capability")));
    }
}
