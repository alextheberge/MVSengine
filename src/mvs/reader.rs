// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{json, Value};

use crate::mvs::manifest::{
    InventoryDiff, LegacyShim, Manifest, ProtocolRange, StringInventoryDiff,
};

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

#[derive(Debug, Clone, Serialize)]
pub struct ManifestComparison {
    pub identity: IdentityComparison,
    pub compatibility: CompatibilityComparison,
    pub capabilities: CapabilityComparison,
    pub ai_contract: AiContractComparison,
    pub environment: EnvironmentComparison,
    pub scan_policy: JsonFieldComparison,
    pub evidence: EvidenceComparison,
}

#[derive(Debug, Clone, Serialize)]
pub struct IdentityComparison {
    pub base: String,
    pub target: String,
    pub arch_delta: i64,
    pub feat_delta: i64,
    pub prot_delta: i64,
    pub context_changed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompatibilityComparison {
    pub host_range_changed: bool,
    pub extension_range_changed: bool,
    pub base_host_range: ProtocolRange,
    pub target_host_range: ProtocolRange,
    pub base_extension_range: ProtocolRange,
    pub target_extension_range: ProtocolRange,
    pub added_legacy_shims: Vec<LegacyShim>,
    pub removed_legacy_shims: Vec<LegacyShim>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityComparison {
    pub changes: Vec<BoolFieldChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiContractComparison {
    pub tool_schema_version_changed: bool,
    pub tool_schema_hash_changed: bool,
    pub prompt_contract_id_changed: bool,
    pub base_tool_schema_version: u64,
    pub target_tool_schema_version: u64,
    pub base_tool_schema_hash: String,
    pub target_tool_schema_hash: String,
    pub base_prompt_contract_id: String,
    pub target_prompt_contract_id: String,
    pub required_model_capabilities: StringInventoryDiff,
    pub provided_model_capabilities: StringInventoryDiff,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvironmentComparison {
    pub profiles: StringInventoryDiff,
    pub runtime_constraints: Vec<StringFieldChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvidenceComparison {
    pub feature_hash_changed: bool,
    pub protocol_hash_changed: bool,
    pub public_api_hash_changed: bool,
    pub diff: InventoryDiff,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonFieldComparison {
    pub changes: Vec<JsonFieldChange>,
}

#[derive(Debug, Clone, Serialize, Eq, PartialEq)]
pub struct BoolFieldChange {
    pub field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Eq, PartialEq)]
pub struct StringFieldChange {
    pub field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct JsonFieldChange {
    pub field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Value>,
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

pub fn compare_manifests(base: &Manifest, target: &Manifest) -> ManifestComparison {
    let identity = IdentityComparison {
        base: base.identity.mvs.clone(),
        target: target.identity.mvs.clone(),
        arch_delta: target.identity.arch as i64 - base.identity.arch as i64,
        feat_delta: target.identity.feat as i64 - base.identity.feat as i64,
        prot_delta: target.identity.prot as i64 - base.identity.prot as i64,
        context_changed: base.identity.cont != target.identity.cont,
    };

    let compatibility = CompatibilityComparison {
        host_range_changed: base.compatibility.host_range != target.compatibility.host_range,
        extension_range_changed: base.compatibility.extension_range
            != target.compatibility.extension_range,
        base_host_range: base.compatibility.host_range.clone(),
        target_host_range: target.compatibility.host_range.clone(),
        base_extension_range: base.compatibility.extension_range.clone(),
        target_extension_range: target.compatibility.extension_range.clone(),
        added_legacy_shims: diff_legacy_shims(
            &base.compatibility.legacy_shims,
            &target.compatibility.legacy_shims,
        )
        .0,
        removed_legacy_shims: diff_legacy_shims(
            &base.compatibility.legacy_shims,
            &target.compatibility.legacy_shims,
        )
        .1,
    };

    let capabilities = CapabilityComparison {
        changes: diff_bool_map(&base.capabilities, &target.capabilities),
    };

    let ai_contract = AiContractComparison {
        tool_schema_version_changed: base.ai_contract.tool_schema_version
            != target.ai_contract.tool_schema_version,
        tool_schema_hash_changed: base.ai_contract.tool_schema_hash
            != target.ai_contract.tool_schema_hash,
        prompt_contract_id_changed: base.ai_contract.prompt_contract_id
            != target.ai_contract.prompt_contract_id,
        base_tool_schema_version: base.ai_contract.tool_schema_version,
        target_tool_schema_version: target.ai_contract.tool_schema_version,
        base_tool_schema_hash: base.ai_contract.tool_schema_hash.clone(),
        target_tool_schema_hash: target.ai_contract.tool_schema_hash.clone(),
        base_prompt_contract_id: base.ai_contract.prompt_contract_id.clone(),
        target_prompt_contract_id: target.ai_contract.prompt_contract_id.clone(),
        required_model_capabilities: diff_strings(
            &base.ai_contract.required_model_capabilities,
            &target.ai_contract.required_model_capabilities,
        ),
        provided_model_capabilities: diff_strings(
            &base.ai_contract.provided_model_capabilities,
            &target.ai_contract.provided_model_capabilities,
        ),
    };

    let environment = EnvironmentComparison {
        profiles: diff_strings(&base.environment.profiles, &target.environment.profiles),
        runtime_constraints: diff_string_map(
            &base.environment.runtime_constraints,
            &target.environment.runtime_constraints,
        ),
    };

    let scan_policy = JsonFieldComparison {
        changes: diff_json_object(
            serde_json::to_value(&base.scan_policy).unwrap_or(Value::Null),
            serde_json::to_value(&target.scan_policy).unwrap_or(Value::Null),
        ),
    };

    let evidence = EvidenceComparison {
        feature_hash_changed: base.evidence.feature_hash != target.evidence.feature_hash,
        protocol_hash_changed: base.evidence.protocol_hash != target.evidence.protocol_hash,
        public_api_hash_changed: base.evidence.public_api_hash != target.evidence.public_api_hash,
        diff: base.evidence.semantic_diff(
            &target.evidence.feature_inventory,
            &target.evidence.protocol_inventory,
            &target.evidence.public_api_inventory,
        ),
    };

    ManifestComparison {
        identity,
        compatibility,
        capabilities,
        ai_contract,
        environment,
        scan_policy,
        evidence,
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

fn diff_strings(base: &[String], target: &[String]) -> StringInventoryDiff {
    let base_set: BTreeSet<&str> = base.iter().map(String::as_str).collect();
    let target_set: BTreeSet<&str> = target.iter().map(String::as_str).collect();

    StringInventoryDiff {
        added: target_set
            .difference(&base_set)
            .map(|item| (*item).to_string())
            .collect(),
        removed: base_set
            .difference(&target_set)
            .map(|item| (*item).to_string())
            .collect(),
    }
}

fn diff_bool_map(
    base: &BTreeMap<String, bool>,
    target: &BTreeMap<String, bool>,
) -> Vec<BoolFieldChange> {
    let mut fields = BTreeSet::new();
    fields.extend(base.keys().cloned());
    fields.extend(target.keys().cloned());

    fields
        .into_iter()
        .filter_map(|field| {
            let base_value = base.get(&field).copied();
            let target_value = target.get(&field).copied();
            (base_value != target_value).then_some(BoolFieldChange {
                field,
                base: base_value,
                target: target_value,
            })
        })
        .collect()
}

fn diff_string_map(
    base: &BTreeMap<String, String>,
    target: &BTreeMap<String, String>,
) -> Vec<StringFieldChange> {
    let mut fields = BTreeSet::new();
    fields.extend(base.keys().cloned());
    fields.extend(target.keys().cloned());

    fields
        .into_iter()
        .filter_map(|field| {
            let base_value = base.get(&field).cloned();
            let target_value = target.get(&field).cloned();
            (base_value != target_value).then_some(StringFieldChange {
                field,
                base: base_value,
                target: target_value,
            })
        })
        .collect()
}

fn diff_json_object(base: Value, target: Value) -> Vec<JsonFieldChange> {
    let base_map = base.as_object().cloned().unwrap_or_default();
    let target_map = target.as_object().cloned().unwrap_or_default();
    let mut fields = BTreeSet::new();
    fields.extend(base_map.keys().cloned());
    fields.extend(target_map.keys().cloned());

    fields
        .into_iter()
        .filter_map(|field| {
            let base_value = base_map.get(&field).cloned();
            let target_value = target_map.get(&field).cloned();
            (base_value != target_value).then_some(JsonFieldChange {
                field,
                base: base_value,
                target: target_value,
            })
        })
        .collect()
}

fn diff_legacy_shims(
    base: &[LegacyShim],
    target: &[LegacyShim],
) -> (Vec<LegacyShim>, Vec<LegacyShim>) {
    let base_set: BTreeSet<&LegacyShim> = base.iter().collect();
    let target_set: BTreeSet<&LegacyShim> = target.iter().collect();

    (
        target_set
            .difference(&base_set)
            .map(|shim| (*shim).clone())
            .collect(),
        base_set
            .difference(&target_set)
            .map(|shim| (*shim).clone())
            .collect(),
    )
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

impl ManifestComparison {
    pub fn changed_sections(&self) -> Vec<&'static str> {
        let mut sections = Vec::new();
        if self.identity.is_changed() {
            sections.push("identity");
        }
        if self.compatibility.is_changed() {
            sections.push("compatibility");
        }
        if self.capabilities.is_changed() {
            sections.push("capabilities");
        }
        if self.ai_contract.is_changed() {
            sections.push("ai_contract");
        }
        if self.environment.is_changed() {
            sections.push("environment");
        }
        if self.scan_policy.is_changed() {
            sections.push("scan_policy");
        }
        if self.evidence.is_changed() {
            sections.push("evidence");
        }
        sections
    }

    pub fn change_count(&self) -> usize {
        self.identity.change_count()
            + self.compatibility.change_count()
            + self.capabilities.change_count()
            + self.ai_contract.change_count()
            + self.environment.change_count()
            + self.scan_policy.change_count()
            + self.evidence.change_count()
    }

    pub fn is_changed(&self) -> bool {
        self.change_count() > 0
    }
}

impl IdentityComparison {
    fn is_changed(&self) -> bool {
        self.arch_delta != 0 || self.feat_delta != 0 || self.prot_delta != 0 || self.context_changed
    }

    fn change_count(&self) -> usize {
        usize::from(self.arch_delta != 0)
            + usize::from(self.feat_delta != 0)
            + usize::from(self.prot_delta != 0)
            + usize::from(self.context_changed)
    }
}

impl CompatibilityComparison {
    fn is_changed(&self) -> bool {
        self.host_range_changed
            || self.extension_range_changed
            || !self.added_legacy_shims.is_empty()
            || !self.removed_legacy_shims.is_empty()
    }

    fn change_count(&self) -> usize {
        usize::from(self.host_range_changed)
            + usize::from(self.extension_range_changed)
            + self.added_legacy_shims.len()
            + self.removed_legacy_shims.len()
    }
}

impl CapabilityComparison {
    fn is_changed(&self) -> bool {
        !self.changes.is_empty()
    }

    fn change_count(&self) -> usize {
        self.changes.len()
    }
}

impl AiContractComparison {
    fn is_changed(&self) -> bool {
        self.tool_schema_version_changed
            || self.tool_schema_hash_changed
            || self.prompt_contract_id_changed
            || !self.required_model_capabilities.is_empty()
            || !self.provided_model_capabilities.is_empty()
    }

    fn change_count(&self) -> usize {
        usize::from(self.tool_schema_version_changed)
            + usize::from(self.tool_schema_hash_changed)
            + usize::from(self.prompt_contract_id_changed)
            + self.required_model_capabilities.added.len()
            + self.required_model_capabilities.removed.len()
            + self.provided_model_capabilities.added.len()
            + self.provided_model_capabilities.removed.len()
    }
}

impl EnvironmentComparison {
    fn is_changed(&self) -> bool {
        !self.profiles.is_empty() || !self.runtime_constraints.is_empty()
    }

    fn change_count(&self) -> usize {
        self.profiles.added.len() + self.profiles.removed.len() + self.runtime_constraints.len()
    }
}

impl EvidenceComparison {
    fn is_changed(&self) -> bool {
        self.feature_hash_changed
            || self.protocol_hash_changed
            || self.public_api_hash_changed
            || !self.diff.is_empty()
    }

    fn change_count(&self) -> usize {
        usize::from(self.feature_hash_changed)
            + usize::from(self.protocol_hash_changed)
            + usize::from(self.public_api_hash_changed)
            + self.diff.features.added.len()
            + self.diff.features.removed.len()
            + self.diff.protocols.added.len()
            + self.diff.protocols.removed.len()
            + self.diff.public_api.added.len()
            + self.diff.public_api.removed.len()
    }
}

impl JsonFieldComparison {
    fn is_changed(&self) -> bool {
        !self.changes.is_empty()
    }

    fn change_count(&self) -> usize {
        self.changes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        compare_manifests, validate_host_extension, ValidationAxis, ValidationCheckStatus,
    };
    use crate::mvs::manifest::{LegacyShim, Manifest, ProtocolRange, PublicApiSnapshot};

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

    #[test]
    fn compare_manifests_reports_identity_policy_and_evidence_changes() {
        let mut base = base_manifest("cli", 1);
        base.identity.feat = 1;
        base.sync_identity_string();
        base.capabilities
            .insert("offline_storage".to_string(), true);
        base.ai_contract.tool_schema_hash = "schema-v1".to_string();
        base.ai_contract.prompt_contract_id = "prompt-v1".to_string();
        base.environment.profiles = vec!["cli".to_string()];
        base.environment
            .runtime_constraints
            .insert("rust".to_string(), "1.74+".to_string());
        base.scan_policy.public_api_roots = vec!["src/api.ts".to_string()];
        base.evidence.feature_hash = "feature-v1".to_string();
        base.evidence.protocol_hash = "protocol-v1".to_string();
        base.evidence.public_api_hash = "api-v1".to_string();
        base.evidence.feature_inventory = vec!["auth_flow".to_string()];
        base.evidence.protocol_inventory = vec!["auth-api-v1".to_string()];
        base.evidence.public_api_inventory = vec![PublicApiSnapshot {
            file: "src/api.ts".to_string(),
            signature: "ts/js:function login(username: string): Promise<string>".to_string(),
        }];

        let mut target = base.clone();
        target.identity.feat = 2;
        target.identity.prot = 2;
        target.sync_identity_string();
        target.compatibility.legacy_shims.push(LegacyShim {
            from_prot: 1,
            to_prot: 2,
            adapter: "auth_v1_to_v2".to_string(),
        });
        target.capabilities.insert("streaming".to_string(), true);
        target.ai_contract.tool_schema_version = 2;
        target.ai_contract.tool_schema_hash = "schema-v2".to_string();
        target.ai_contract.prompt_contract_id = "prompt-v2".to_string();
        target
            .ai_contract
            .required_model_capabilities
            .push("json_schema".to_string());
        target.environment.profiles.push("edge".to_string());
        target
            .environment
            .runtime_constraints
            .insert("node".to_string(), "20+".to_string());
        target.scan_policy.public_api_excludes = vec!["ts/js:const buildSession".to_string()];
        target.evidence.feature_hash = "feature-v2".to_string();
        target.evidence.protocol_hash = "protocol-v2".to_string();
        target.evidence.public_api_hash = "api-v2".to_string();
        target
            .evidence
            .feature_inventory
            .push("offline_storage".to_string());
        target
            .evidence
            .protocol_inventory
            .push("token_handshake".to_string());
        target
            .evidence
            .public_api_inventory
            .push(PublicApiSnapshot {
                file: "src/api.ts".to_string(),
                signature: "ts/js:interface Session".to_string(),
            });

        let comparison = compare_manifests(&base, &target);

        assert!(comparison.is_changed());
        assert!(comparison.changed_sections().contains(&"identity"));
        assert!(comparison.changed_sections().contains(&"scan_policy"));
        assert!(comparison.changed_sections().contains(&"evidence"));
        assert_eq!(comparison.identity.feat_delta, 1);
        assert_eq!(comparison.identity.prot_delta, 1);
        assert_eq!(comparison.compatibility.added_legacy_shims.len(), 1);
        assert_eq!(comparison.capabilities.changes.len(), 1);
        assert!(comparison.ai_contract.tool_schema_version_changed);
        assert_eq!(
            comparison.environment.profiles.added,
            vec!["edge".to_string()]
        );
        assert_eq!(comparison.scan_policy.changes.len(), 1);
        assert_eq!(
            comparison.evidence.diff.public_api.added,
            vec![PublicApiSnapshot {
                file: "src/api.ts".to_string(),
                signature: "ts/js:interface Session".to_string(),
            }]
        );
    }
}
