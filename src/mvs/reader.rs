// SPDX-License-Identifier: AGPL-3.0-only
use serde::Serialize;
use serde_json::{json, Value};

use crate::mvs::manifest::Manifest;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationAxis {
    Protocol,
    Context,
    RuntimeProfile,
    Capabilities,
    AiSchema,
    AiModelCapabilities,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationCheckStatus {
    Pass,
    Degraded,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationCheck {
    pub axis: ValidationAxis,
    pub status: ValidationCheckStatus,
    pub code: &'static str,
    pub message: String,
    #[serde(default, skip_serializing_if = "validation_details_is_empty")]
    pub details: Value,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ValidationResult {
    pub compatible: bool,
    pub degraded: bool,
    pub reasons: Vec<String>,
    pub checks: Vec<ValidationCheck>,
}

pub fn validate_host_extension(
    host: &Manifest,
    extension: &Manifest,
    context: Option<&str>,
    allow_shims: bool,
    host_model_capabilities_override: Option<&[String]>,
) -> ValidationResult {
    let mut reasons = Vec::new();
    let mut checks = Vec::new();
    let mut compatible = true;
    let mut degraded = false;

    let host_prot = host.identity.prot;
    let extension_prot = extension.identity.prot;

    let host_accepts_extension = host.compatibility.extension_range.contains(extension_prot);
    let extension_accepts_host = extension.compatibility.host_range.contains(host_prot);

    if !host_accepts_extension || !extension_accepts_host {
        if let Some(shim_adapter) = allow_shims
            .then(|| matching_shim(host, extension_prot, host_prot))
            .flatten()
        {
            degraded = true;
            let message = format!(
                "Protocol range mismatch resolved through legacy shim (extension PROT {}, host PROT {}).",
                extension_prot, host_prot
            );
            reasons.push(message.clone());
            checks.push(ValidationCheck::degraded(
                ValidationAxis::Protocol,
                "protocol_range_shimmed",
                message,
                json!({
                    "extension_protocol": extension_prot,
                    "host_protocol": host_prot,
                    "shim_adapter": shim_adapter,
                }),
            ));
        } else {
            compatible = false;
            let mut details = json!({
                "extension_protocol": extension_prot,
                "host_protocol": host_prot,
                "required_host_range": {
                    "min_prot": extension.compatibility.host_range.min_prot,
                    "max_prot": extension.compatibility.host_range.max_prot,
                },
                "host_extension_range": {
                    "min_prot": host.compatibility.extension_range.min_prot,
                    "max_prot": host.compatibility.extension_range.max_prot,
                },
            });
            let message = format!(
                "Protocol range mismatch: extension requires host {}-{}, host exposes {}-{} and is at PROT {}.",
                extension.compatibility.host_range.min_prot,
                extension.compatibility.host_range.max_prot,
                host.compatibility.extension_range.min_prot,
                host.compatibility.extension_range.max_prot,
                host_prot
            );
            reasons.push(message.clone());

            if let Some(reason) = extension.latest_protocol_reason(extension_prot) {
                details["extension_protocol_reason"] = json!(reason);
                reasons.push(format!(
                    "Extension protocol {} rationale: {}",
                    extension_prot, reason
                ));
            }
            if let Some(reason) = host.latest_protocol_reason(host_prot) {
                details["host_protocol_reason"] = json!(reason);
                reasons.push(format!("Host protocol {} rationale: {}", host_prot, reason));
            }

            checks.push(ValidationCheck::fail(
                ValidationAxis::Protocol,
                "protocol_range_mismatch",
                message,
                details,
            ));
        }
    } else {
        checks.push(ValidationCheck::pass(
            ValidationAxis::Protocol,
            "protocol_range_ok",
            format!(
                "Protocol ranges are compatible (extension PROT {}, host PROT {}).",
                extension_prot, host_prot
            ),
            json!({
                "extension_protocol": extension_prot,
                "host_protocol": host_prot,
            }),
        ));
    }

    let target_context = context.unwrap_or(&extension.identity.cont);
    if !context_satisfies(target_context, &extension.identity.cont) {
        compatible = false;
        let message = format!(
            "Context mismatch: extension CONT `{}` is not compatible with target `{}`.",
            extension.identity.cont, target_context
        );
        reasons.push(message.clone());
        checks.push(ValidationCheck::fail(
            ValidationAxis::Context,
            "context_mismatch",
            message,
            json!({
                "required_context": extension.identity.cont,
                "target_context": target_context,
            }),
        ));
    } else {
        checks.push(ValidationCheck::pass(
            ValidationAxis::Context,
            "context_ok",
            format!(
                "Target context `{}` satisfies extension CONT `{}`.",
                target_context, extension.identity.cont
            ),
            json!({
                "required_context": extension.identity.cont,
                "target_context": target_context,
            }),
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
        let message = format!(
            "Host runtime profiles do not support target context `{}`.",
            target_context
        );
        reasons.push(message.clone());
        checks.push(ValidationCheck::fail(
            ValidationAxis::RuntimeProfile,
            "runtime_profile_unsupported",
            message,
            json!({
                "target_context": target_context,
                "host_profiles": host.environment.profiles,
            }),
        ));
    } else {
        checks.push(ValidationCheck::pass(
            ValidationAxis::RuntimeProfile,
            "runtime_profile_ok",
            if host.environment.profiles.is_empty() {
                format!(
                    "Host does not restrict runtime profiles for target context `{}`.",
                    target_context
                )
            } else {
                format!(
                    "Host runtime profiles support target context `{}`.",
                    target_context
                )
            },
            json!({
                "target_context": target_context,
                "host_profiles": host.environment.profiles,
            }),
        ));
    }

    let missing_capabilities: Vec<String> = extension
        .capabilities
        .iter()
        .filter(|(capability, required)| {
            **required && !host.capabilities.get(*capability).copied().unwrap_or(false)
        })
        .map(|(capability, _)| capability.clone())
        .collect();

    for (capability, required) in &extension.capabilities {
        if *required && !host.capabilities.get(capability).copied().unwrap_or(false) {
            compatible = false;
            reasons.push(format!(
                "Missing required capability: host does not provide `{}`.",
                capability
            ));
        }
    }

    if missing_capabilities.is_empty() {
        checks.push(ValidationCheck::pass(
            ValidationAxis::Capabilities,
            "required_capabilities_ok",
            "Host provides all required extension capabilities.".to_string(),
            json!({
                "required_capabilities": extension
                    .capabilities
                    .iter()
                    .filter_map(|(capability, required)| (*required).then_some(capability))
                    .collect::<Vec<_>>(),
            }),
        ));
    } else {
        checks.push(ValidationCheck::fail(
            ValidationAxis::Capabilities,
            "missing_required_capabilities",
            format!(
                "Host runtime is missing required extension capabilities: {}.",
                missing_capabilities.join(", ")
            ),
            json!({
                "missing_capabilities": missing_capabilities,
            }),
        ));
    }

    if extension.ai_contract.tool_schema_version > host.ai_contract.tool_schema_version {
        compatible = false;
        let message = format!(
            "AI contract schema version unsupported: extension requires {}, host provides {}.",
            extension.ai_contract.tool_schema_version, host.ai_contract.tool_schema_version
        );
        reasons.push(message.clone());
        checks.push(ValidationCheck::fail(
            ValidationAxis::AiSchema,
            "ai_schema_version_unsupported",
            message,
            json!({
                "required_tool_schema_version": extension.ai_contract.tool_schema_version,
                "provided_tool_schema_version": host.ai_contract.tool_schema_version,
            }),
        ));
    } else {
        checks.push(ValidationCheck::pass(
            ValidationAxis::AiSchema,
            "ai_schema_version_ok",
            format!(
                "Host AI schema version {} satisfies extension requirement {}.",
                host.ai_contract.tool_schema_version, extension.ai_contract.tool_schema_version
            ),
            json!({
                "required_tool_schema_version": extension.ai_contract.tool_schema_version,
                "provided_tool_schema_version": host.ai_contract.tool_schema_version,
            }),
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
        let message = format!(
            "AI capability mismatch: host runtime is missing required model capabilities: {}.",
            missing_ai_capabilities.join(", ")
        );
        reasons.push(message.clone());
        checks.push(ValidationCheck::fail(
            ValidationAxis::AiModelCapabilities,
            "missing_ai_model_capabilities",
            message,
            json!({
                "missing_model_capabilities": missing_ai_capabilities,
                "required_model_capabilities": extension.ai_contract.required_model_capabilities,
                "provided_model_capabilities": host_runtime_capabilities,
            }),
        ));
    } else {
        checks.push(ValidationCheck::pass(
            ValidationAxis::AiModelCapabilities,
            "ai_model_capabilities_ok",
            "Host runtime provides all required AI model capabilities.".to_string(),
            json!({
                "required_model_capabilities": extension.ai_contract.required_model_capabilities,
                "provided_model_capabilities": host_runtime_capabilities,
            }),
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
        checks,
    }
}

fn matching_shim(host: &Manifest, from_prot: u64, to_prot: u64) -> Option<&str> {
    host.compatibility
        .legacy_shims
        .iter()
        .find(|shim| shim.from_prot == from_prot && shim.to_prot == to_prot)
        .map(|shim| shim.adapter.as_str())
}

fn context_satisfies(actual: &str, required: &str) -> bool {
    actual == required || actual.starts_with(&format!("{required}."))
}

fn context_pair_compatible(left: &str, right: &str) -> bool {
    context_satisfies(left, right) || context_satisfies(right, left)
}

fn validation_details_is_empty(value: &Value) -> bool {
    matches!(value, Value::Object(map) if map.is_empty())
}

impl ValidationCheck {
    fn pass(axis: ValidationAxis, code: &'static str, message: String, details: Value) -> Self {
        Self {
            axis,
            status: ValidationCheckStatus::Pass,
            code,
            message,
            details,
        }
    }

    fn degraded(axis: ValidationAxis, code: &'static str, message: String, details: Value) -> Self {
        Self {
            axis,
            status: ValidationCheckStatus::Degraded,
            code,
            message,
            details,
        }
    }

    fn fail(axis: ValidationAxis, code: &'static str, message: String, details: Value) -> Self {
        Self {
            axis,
            status: ValidationCheckStatus::Fail,
            code,
            message,
            details,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_host_extension, ValidationAxis, ValidationCheckStatus};
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
        assert!(result.checks.iter().any(|check| {
            check.axis == ValidationAxis::Protocol
                && check.status == ValidationCheckStatus::Pass
                && check.code == "protocol_range_ok"
        }));
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
        assert!(result.checks.iter().any(|check| {
            check.axis == ValidationAxis::Protocol
                && check.status == ValidationCheckStatus::Degraded
                && check.code == "protocol_range_shimmed"
        }));
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
        assert!(result.checks.iter().any(|check| {
            check.axis == ValidationAxis::Capabilities
                && check.status == ValidationCheckStatus::Fail
                && check.code == "missing_required_capabilities"
        }));
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
        assert!(result.checks.iter().any(|check| {
            check.axis == ValidationAxis::AiModelCapabilities
                && check.status == ValidationCheckStatus::Fail
                && check.code == "missing_ai_model_capabilities"
        }));
    }
}
