// SPDX-License-Identifier: AGPL-3.0-only
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(rename = "$schema")]
    pub schema: String,

    pub identity: Identity,

    #[serde(default)]
    pub compatibility: Compatibility,

    #[serde(default)]
    pub capabilities: BTreeMap<String, bool>,

    #[serde(default)]
    pub ai_contract: AiContract,

    #[serde(default)]
    pub environment: Environment,

    #[serde(default, skip_serializing_if = "ScanPolicy::is_empty")]
    pub scan_policy: ScanPolicy,

    #[serde(default)]
    pub evidence: Evidence,

    #[serde(default)]
    pub history: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub mvs: String,
    pub arch: u64,
    pub feat: u64,
    pub prot: u64,
    pub cont: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Compatibility {
    #[serde(default)]
    pub host_range: ProtocolRange,

    #[serde(default)]
    pub extension_range: ProtocolRange,

    #[serde(default)]
    pub legacy_shims: Vec<LegacyShim>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProtocolRange {
    pub min_prot: u64,
    pub max_prot: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyShim {
    pub from_prot: u64,
    pub to_prot: u64,
    pub adapter: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiContract {
    pub tool_schema_version: u64,
    pub tool_schema_hash: String,
    #[serde(default)]
    pub required_model_capabilities: Vec<String>,
    #[serde(default)]
    pub provided_model_capabilities: Vec<String>,
    pub prompt_contract_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Environment {
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub runtime_constraints: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScanPolicy {
    #[serde(default)]
    pub exclude_paths: Vec<String>,
    #[serde(default)]
    pub public_api_roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Evidence {
    pub feature_hash: String,
    pub protocol_hash: String,
    pub public_api_hash: String,
    #[serde(default)]
    pub feature_inventory: Vec<String>,
    #[serde(default)]
    pub protocol_inventory: Vec<String>,
    #[serde(default)]
    pub public_api_inventory: Vec<PublicApiSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub mvs: String,
    pub arch: u64,
    pub feat: u64,
    pub prot: u64,
    pub cont: String,
    pub reasons: Vec<String>,
    pub changed_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct PublicApiSnapshot {
    pub file: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Default, Eq, PartialEq)]
pub struct InventoryDiff {
    pub features: StringInventoryDiff,
    pub protocols: StringInventoryDiff,
    pub public_api: PublicApiInventoryDiff,
}

#[derive(Debug, Clone, Serialize, Default, Eq, PartialEq)]
pub struct StringInventoryDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default, Eq, PartialEq)]
pub struct PublicApiInventoryDiff {
    pub added: Vec<PublicApiSnapshot>,
    pub removed: Vec<PublicApiSnapshot>,
}

impl ProtocolRange {
    pub fn contains(&self, prot: u64) -> bool {
        prot >= self.min_prot && prot <= self.max_prot
    }
}

impl Default for AiContract {
    fn default() -> Self {
        Self {
            tool_schema_version: 1,
            tool_schema_hash: String::new(),
            required_model_capabilities: Vec::new(),
            provided_model_capabilities: Vec::new(),
            prompt_contract_id: "default".to_string(),
        }
    }
}

impl Manifest {
    pub fn default_for_context(context: &str) -> Self {
        let mut manifest = Self {
            schema: "https://mvs.dev/schema/v1".to_string(),
            identity: Identity {
                mvs: String::new(),
                arch: 0,
                feat: 0,
                prot: 0,
                cont: context.to_string(),
            },
            compatibility: Compatibility::default(),
            capabilities: BTreeMap::new(),
            ai_contract: AiContract::default(),
            environment: Environment::default(),
            scan_policy: ScanPolicy::default(),
            evidence: Evidence::default(),
            history: Vec::new(),
        };

        manifest.sync_identity_string();
        manifest.compatibility.host_range = ProtocolRange {
            min_prot: manifest.identity.prot,
            max_prot: manifest.identity.prot,
        };
        manifest.compatibility.extension_range = ProtocolRange {
            min_prot: manifest.identity.prot,
            max_prot: manifest.identity.prot,
        };
        manifest.environment.profiles.push(context.to_string());

        manifest
    }

    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read manifest: {}", path.display()))?;
        let manifest: Self = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse manifest: {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn load_if_exists(path: &Path, context: &str) -> Result<Self> {
        if path.exists() {
            return Self::load(path);
        }

        Ok(Self::default_for_context(context))
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let formatted =
            serde_json::to_string_pretty(self).context("failed to serialize manifest")?;
        fs::write(path, format!("{formatted}\n"))
            .with_context(|| format!("failed to write manifest: {}", path.display()))
    }

    pub fn sync_identity_string(&mut self) {
        self.identity.mvs = format!(
            "{}.{}.{}-{}",
            self.identity.arch, self.identity.feat, self.identity.prot, self.identity.cont
        );
    }

    pub fn validate(&self) -> Result<()> {
        if self.identity.cont.trim().is_empty() {
            bail!("identity.cont must be non-empty");
        }

        let expected = format!(
            "{}.{}.{}-{}",
            self.identity.arch, self.identity.feat, self.identity.prot, self.identity.cont
        );
        if self.identity.mvs != expected {
            bail!(
                "identity.mvs mismatch: found `{}`, expected `{}`",
                self.identity.mvs,
                expected
            );
        }

        validate_range("compatibility.host_range", &self.compatibility.host_range)?;
        validate_range(
            "compatibility.extension_range",
            &self.compatibility.extension_range,
        )?;
        self.scan_policy.validate()?;

        Ok(())
    }

    pub fn append_history_entry(&mut self, reasons: Vec<String>) {
        if reasons.is_empty() {
            return;
        }

        let entry = HistoryEntry {
            mvs: self.identity.mvs.clone(),
            arch: self.identity.arch,
            feat: self.identity.feat,
            prot: self.identity.prot,
            cont: self.identity.cont.clone(),
            reasons,
            changed_at_unix: current_unix_timestamp(),
        };
        self.history.push(entry);
    }

    pub fn latest_protocol_reason(&self, prot: u64) -> Option<String> {
        self.history.iter().rev().find_map(|entry| {
            if entry.prot != prot {
                return None;
            }

            entry
                .reasons
                .iter()
                .find(|reason| reason.to_ascii_lowercase().contains("protocol"))
                .cloned()
                .or_else(|| entry.reasons.first().cloned())
        })
    }
}

impl ScanPolicy {
    pub fn is_empty(&self) -> bool {
        self.exclude_paths.is_empty() && self.public_api_roots.is_empty()
    }

    pub fn is_excluded(&self, relative_path: &str) -> bool {
        self.exclude_paths
            .iter()
            .any(|pattern| path_matches(relative_path, pattern))
    }

    pub fn includes_public_api(&self, relative_path: &str) -> bool {
        self.public_api_roots.is_empty()
            || self
                .public_api_roots
                .iter()
                .any(|pattern| path_matches(relative_path, pattern))
    }

    pub fn validate(&self) -> Result<()> {
        validate_policy_paths("scan_policy.exclude_paths", &self.exclude_paths)?;
        validate_policy_paths("scan_policy.public_api_roots", &self.public_api_roots)?;
        Ok(())
    }
}

impl Evidence {
    pub fn semantic_diff(
        &self,
        feature_inventory: &[String],
        protocol_inventory: &[String],
        public_api_inventory: &[PublicApiSnapshot],
    ) -> InventoryDiff {
        InventoryDiff {
            features: diff_strings(&self.feature_inventory, feature_inventory),
            protocols: diff_strings(&self.protocol_inventory, protocol_inventory),
            public_api: diff_public_api(&self.public_api_inventory, public_api_inventory),
        }
    }
}

impl InventoryDiff {
    pub fn is_empty(&self) -> bool {
        self.features.is_empty() && self.protocols.is_empty() && self.public_api.is_empty()
    }
}

impl StringInventoryDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

impl PublicApiInventoryDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

fn diff_strings(previous: &[String], current: &[String]) -> StringInventoryDiff {
    let previous_set: BTreeSet<&str> = previous.iter().map(String::as_str).collect();
    let current_set: BTreeSet<&str> = current.iter().map(String::as_str).collect();

    StringInventoryDiff {
        added: current_set
            .difference(&previous_set)
            .map(|item| (*item).to_string())
            .collect(),
        removed: previous_set
            .difference(&current_set)
            .map(|item| (*item).to_string())
            .collect(),
    }
}

fn diff_public_api(
    previous: &[PublicApiSnapshot],
    current: &[PublicApiSnapshot],
) -> PublicApiInventoryDiff {
    let previous_set: BTreeSet<&PublicApiSnapshot> = previous.iter().collect();
    let current_set: BTreeSet<&PublicApiSnapshot> = current.iter().collect();

    PublicApiInventoryDiff {
        added: current_set
            .difference(&previous_set)
            .map(|item| (*item).clone())
            .collect(),
        removed: previous_set
            .difference(&current_set)
            .map(|item| (*item).clone())
            .collect(),
    }
}

fn validate_policy_paths(label: &str, paths: &[String]) -> Result<()> {
    for path in paths {
        if Path::new(path.trim()).is_absolute() {
            bail!("{label} entries must be relative paths, found `{path}`");
        }
        let normalized = normalize_policy_path(path);
        if normalized.is_empty() {
            bail!("{label} contains an empty path entry");
        }
    }

    Ok(())
}

fn normalize_policy_path(path: &str) -> String {
    let normalized = path.trim().replace('\\', "/");
    let normalized = normalized.trim_start_matches("./").trim_matches('/');
    normalized.to_string()
}

fn path_matches(relative_path: &str, pattern: &str) -> bool {
    let normalized_path = normalize_policy_path(relative_path);
    let normalized_pattern = normalize_policy_path(pattern);

    normalized_path == normalized_pattern
        || normalized_path.starts_with(&(normalized_pattern + "/"))
}

fn validate_range(label: &str, range: &ProtocolRange) -> Result<()> {
    if range.min_prot > range.max_prot {
        bail!(
            "{} invalid: min_prot ({}) must be <= max_prot ({})",
            label,
            range.min_prot,
            range.max_prot
        );
    }

    Ok(())
}

fn current_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{Evidence, Manifest, ProtocolRange, PublicApiSnapshot, ScanPolicy};

    #[test]
    fn default_manifest_has_valid_identity() {
        let manifest = Manifest::default_for_context("cli");
        assert_eq!(manifest.identity.mvs, "0.0.0-cli");
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn validation_fails_when_identity_string_is_out_of_sync() {
        let mut manifest = Manifest::default_for_context("web");
        manifest.identity.feat = 2;
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn validation_fails_for_invalid_protocol_range() {
        let mut manifest = Manifest::default_for_context("edge");
        manifest.compatibility.host_range = ProtocolRange {
            min_prot: 10,
            max_prot: 2,
        };
        manifest.sync_identity_string();

        assert!(manifest.validate().is_err());
    }

    #[test]
    fn history_entries_can_be_recorded_and_queried() {
        let mut manifest = Manifest::default_for_context("cli");
        manifest.identity.prot = 12;
        manifest.sync_identity_string();
        manifest.append_history_entry(vec![
            "Protocol incremented due to Auth Flow break in /src/api/auth.ts.".to_string(),
        ]);

        let reason = manifest.latest_protocol_reason(12);
        assert!(reason.is_some());
        assert!(reason.unwrap_or_default().contains("Auth Flow break"));
    }

    #[test]
    fn semantic_diff_reports_added_and_removed_inventory_items() {
        let evidence = Evidence {
            feature_hash: "f".to_string(),
            protocol_hash: "p".to_string(),
            public_api_hash: "a".to_string(),
            feature_inventory: vec!["offline_storage".to_string()],
            protocol_inventory: vec!["cli_generate_command".to_string()],
            public_api_inventory: vec![PublicApiSnapshot {
                file: "src/lib.rs".to_string(),
                signature: "rust:fn login(user:String)".to_string(),
            }],
        };

        let diff = evidence.semantic_diff(
            &["sync".to_string()],
            &[
                "cli_generate_command".to_string(),
                "cli_lint_command".to_string(),
            ],
            &[PublicApiSnapshot {
                file: "src/lib.rs".to_string(),
                signature: "rust:fn rotate_token(token:String)".to_string(),
            }],
        );

        assert_eq!(diff.features.added, vec!["sync".to_string()]);
        assert_eq!(diff.features.removed, vec!["offline_storage".to_string()]);
        assert_eq!(diff.protocols.added, vec!["cli_lint_command".to_string()]);
        assert_eq!(diff.protocols.removed, Vec::<String>::new());
        assert_eq!(diff.public_api.added.len(), 1);
        assert_eq!(diff.public_api.removed.len(), 1);
    }

    #[test]
    fn scan_policy_matches_relative_paths_and_prefixes() {
        let policy = ScanPolicy {
            exclude_paths: vec!["src/generated".to_string()],
            public_api_roots: vec!["src/cli.rs".to_string(), "src/facade".to_string()],
        };

        assert!(policy.is_excluded("src/generated/client.rs"));
        assert!(!policy.is_excluded("src/cli.rs"));
        assert!(policy.includes_public_api("src/cli.rs"));
        assert!(policy.includes_public_api("src/facade/mod.rs"));
        assert!(!policy.includes_public_api("src/internal/mod.rs"));
    }

    #[test]
    fn validation_rejects_absolute_scan_policy_paths() {
        let mut manifest = Manifest::default_for_context("cli");
        manifest.scan_policy.exclude_paths = vec!["/tmp/generated".to_string()];

        assert!(manifest.validate().is_err());
    }
}
