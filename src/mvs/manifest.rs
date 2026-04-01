// SPDX-License-Identifier: AGPL-3.0-only
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::mvs::hashing::hash_items;

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_api_roots: Vec<String>,
    #[serde(default, skip_serializing_if = "RubyExportFollowing::is_default")]
    pub ruby_export_following: RubyExportFollowing,
    #[serde(default, skip_serializing_if = "LuaExportFollowing::is_default")]
    pub lua_export_following: LuaExportFollowing,
    #[serde(default, skip_serializing_if = "PythonExportFollowing::is_default")]
    pub python_export_following: PythonExportFollowing,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub python_module_roots: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_api_includes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_api_excludes: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PythonExportFollowing {
    Off,
    RootsOnly,
    #[default]
    Heuristic,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RubyExportFollowing {
    Off,
    #[default]
    Heuristic,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LuaExportFollowing {
    Off,
    ReturnedRootOnly,
    #[default]
    Heuristic,
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
        let mut manifest: Self = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse manifest: {}", path.display()))?;
        manifest.evidence = manifest.evidence.canonicalized();
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
        self.exclude_paths.is_empty()
            && self.public_api_roots.is_empty()
            && self.ruby_export_following.is_default()
            && self.lua_export_following.is_default()
            && self.python_export_following.is_default()
            && self.python_module_roots.is_empty()
            && self.public_api_includes.is_empty()
            && self.public_api_excludes.is_empty()
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

    pub fn includes_public_api_item(&self, relative_path: &str, signature: &str) -> bool {
        if self
            .public_api_excludes
            .iter()
            .any(|pattern| public_api_rule_matches(pattern, relative_path, signature))
        {
            return false;
        }

        self.public_api_includes.is_empty()
            || self
                .public_api_includes
                .iter()
                .any(|pattern| public_api_rule_matches(pattern, relative_path, signature))
    }

    pub fn validate(&self) -> Result<()> {
        validate_policy_paths("scan_policy.exclude_paths", &self.exclude_paths)?;
        validate_policy_paths("scan_policy.public_api_roots", &self.public_api_roots)?;
        validate_policy_paths("scan_policy.python_module_roots", &self.python_module_roots)?;
        validate_policy_patterns("scan_policy.public_api_includes", &self.public_api_includes)?;
        validate_policy_patterns("scan_policy.public_api_excludes", &self.public_api_excludes)?;
        if self.python_export_following == PythonExportFollowing::RootsOnly
            && self.python_module_roots.is_empty()
        {
            bail!(
                "scan_policy.python_export_following=roots_only requires scan_policy.python_module_roots"
            );
        }
        Ok(())
    }
}

impl PythonExportFollowing {
    pub fn is_default(&self) -> bool {
        *self == Self::Heuristic
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::RootsOnly => "roots_only",
            Self::Heuristic => "heuristic",
        }
    }
}

impl RubyExportFollowing {
    pub fn is_default(&self) -> bool {
        *self == Self::Heuristic
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Heuristic => "heuristic",
        }
    }
}

impl LuaExportFollowing {
    pub fn is_default(&self) -> bool {
        *self == Self::Heuristic
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::ReturnedRootOnly => "returned_root_only",
            Self::Heuristic => "heuristic",
        }
    }
}

impl Evidence {
    pub fn canonicalized(&self) -> Self {
        let mut evidence = self.clone();
        evidence.public_api_inventory =
            canonicalize_public_api_inventory(&evidence.public_api_inventory);
        if !evidence.public_api_inventory.is_empty() {
            evidence.public_api_hash = hash_public_api_inventory(&evidence.public_api_inventory);
        }
        evidence
    }

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

fn canonicalize_public_api_inventory(inventory: &[PublicApiSnapshot]) -> Vec<PublicApiSnapshot> {
    let mut canonical: Vec<PublicApiSnapshot> = inventory
        .iter()
        .map(|item| PublicApiSnapshot {
            file: normalize_policy_path(&item.file),
            signature: canonicalize_public_api_signature(&item.signature),
        })
        .collect();
    canonical.sort();
    canonical.dedup();
    canonical
}

fn hash_public_api_inventory(inventory: &[PublicApiSnapshot]) -> String {
    hash_items(
        inventory
            .iter()
            .map(|item| format!("{}|{}", item.file, item.signature)),
    )
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

fn validate_policy_patterns(label: &str, patterns: &[String]) -> Result<()> {
    for pattern in patterns {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            bail!("{label} contains an empty pattern entry");
        }

        if let Some((path_pattern, signature_pattern)) = trimmed.split_once('|') {
            if path_pattern.trim().is_empty() || signature_pattern.trim().is_empty() {
                bail!(
                    "{label} selector patterns must be `relative/path|signature-pattern`, found `{pattern}`"
                );
            }
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

fn public_api_rule_matches(pattern: &str, relative_path: &str, signature: &str) -> bool {
    let trimmed = pattern.trim();
    let normalized_path = normalize_policy_path(relative_path);
    let canonical_signature = canonicalize_public_api_signature(signature);
    if let Some((path_pattern, signature_pattern)) = trimmed.split_once('|') {
        wildcard_matches(&normalize_policy_path(path_pattern), &normalized_path)
            && wildcard_matches(
                &canonicalize_public_api_signature(signature_pattern.trim()),
                &canonical_signature,
            )
    } else {
        wildcard_matches(
            &canonicalize_public_api_signature(trimmed),
            &canonical_signature,
        )
    }
}

fn canonicalize_public_api_signature(signature: &str) -> String {
    let mut canonical = signature.trim().to_string();

    if let Some(rest) = canonical.strip_prefix("rust:fn fn ") {
        canonical = format!("rust:fn {rest}");
    } else if let Some(rest) = canonical.strip_prefix("rust:impl-fn ") {
        canonical = format!("rust:impl-fn {}", rest.replacen("::fn ", "::", 1));
    } else if let Some(rest) = canonical.strip_prefix("rust:trait-fn ") {
        canonical = format!("rust:trait-fn {}", rest.replacen("::fn ", "::", 1));
    }

    for (from, to) in [
        ("& mut self", "&mut self"),
        ("& self", "&self"),
        (":&mut", ": &mut"),
        (":&", ": &"),
        ("< ", "<"),
        (" >", ">"),
    ] {
        canonical = canonical.replace(from, to);
    }

    canonical.trim_end_matches(',').to_string()
}

fn wildcard_matches(pattern: &str, candidate: &str) -> bool {
    let pattern = pattern.as_bytes();
    let candidate = candidate.as_bytes();

    let mut pattern_index = 0usize;
    let mut candidate_index = 0usize;
    let mut star_index = None;
    let mut match_index = 0usize;

    while candidate_index < candidate.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'*'
                || pattern[pattern_index] == candidate[candidate_index])
        {
            if pattern[pattern_index] == b'*' {
                star_index = Some(pattern_index);
                match_index = candidate_index;
                pattern_index += 1;
            } else {
                pattern_index += 1;
                candidate_index += 1;
            }
        } else if let Some(star) = star_index {
            pattern_index = star + 1;
            match_index += 1;
            candidate_index = match_index;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }

    pattern_index == pattern.len()
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
    use super::{
        Evidence, LuaExportFollowing, Manifest, ProtocolRange, PublicApiSnapshot,
        PythonExportFollowing, RubyExportFollowing, ScanPolicy,
    };

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
            ruby_export_following: RubyExportFollowing::Heuristic,
            lua_export_following: LuaExportFollowing::Heuristic,
            python_export_following: PythonExportFollowing::Heuristic,
            python_module_roots: vec!["src/python".to_string()],
            public_api_includes: Vec::new(),
            public_api_excludes: Vec::new(),
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

    #[test]
    fn scan_policy_filters_public_api_signatures_by_signature_or_selector() {
        let policy = ScanPolicy {
            exclude_paths: Vec::new(),
            public_api_roots: vec!["src/cli.rs".to_string()],
            ruby_export_following: RubyExportFollowing::Heuristic,
            lua_export_following: LuaExportFollowing::Heuristic,
            python_export_following: PythonExportFollowing::Heuristic,
            python_module_roots: Vec::new(),
            public_api_includes: vec![
                "rust:fn *".to_string(),
                "src/cli.rs|rust:enum OutputFormat".to_string(),
            ],
            public_api_excludes: vec!["rust:fn fn internal_*".to_string()],
        };

        assert!(policy.includes_public_api_item("src/cli.rs", "rust:fn run() -> i32"));
        assert!(policy.includes_public_api_item("src/cli.rs", "rust:enum OutputFormat"));
        assert!(!policy.includes_public_api_item("src/cli.rs", "rust:fn internal_probe() -> i32"));
        assert!(!policy.includes_public_api_item("src/cli.rs", "rust:struct GenerateArgs"));
        assert!(!policy.includes_public_api_item("src/internal.rs", "rust:enum OutputFormat"));
    }

    #[test]
    fn validation_rejects_empty_public_api_selector_patterns() {
        let mut manifest = Manifest::default_for_context("cli");
        manifest.scan_policy.public_api_excludes = vec!["src/cli.rs|".to_string()];

        assert!(manifest.validate().is_err());
    }

    #[test]
    fn validation_rejects_roots_only_python_following_without_roots() {
        let mut manifest = Manifest::default_for_context("cli");
        manifest.scan_policy.python_export_following = PythonExportFollowing::RootsOnly;

        assert!(manifest.validate().is_err());
    }

    #[test]
    fn evidence_canonicalization_migrates_legacy_rust_signatures() {
        let evidence = Evidence {
            feature_hash: "f".to_string(),
            protocol_hash: "p".to_string(),
            public_api_hash: "legacy".to_string(),
            feature_inventory: Vec::new(),
            protocol_inventory: Vec::new(),
            public_api_inventory: vec![
                PublicApiSnapshot {
                    file: "./src/cli.rs".to_string(),
                    signature: "rust:fn fn run() -> i32".to_string(),
                },
                PublicApiSnapshot {
                    file: "src/lib.rs".to_string(),
                    signature: "rust:impl-fn HostAdapter::fn connect(& self, target:&str) -> bool"
                        .to_string(),
                },
            ],
        };

        let canonical = evidence.canonicalized();

        assert_eq!(canonical.public_api_inventory[0].file, "src/cli.rs");
        assert_eq!(
            canonical.public_api_inventory[0].signature,
            "rust:fn run() -> i32"
        );
        assert_eq!(
            canonical.public_api_inventory[1].signature,
            "rust:impl-fn HostAdapter::connect(&self, target: &str) -> bool"
        );
        assert_ne!(canonical.public_api_hash, "legacy");
    }
}
