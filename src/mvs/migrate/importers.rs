// SPDX-License-Identifier: AGPL-3.0-only
//! Reads existing release-tooling configuration so a migration can carry
//! over settings instead of starting from nothing.
//!
//! Two formats get a real import: `.bumpversion.cfg` / `.bumpversion.toml`
//! and `tbump.toml` both declare exactly which files contain the version,
//! which maps directly onto `release.version_files`. Everything else
//! (semantic-release, release-please, changesets, GitVersion,
//! Nerdbank.GitVersioning, cargo-release, goreleaser, lerna) is detected by
//! config-file presence only — they either compute versions from commit
//! history rather than declaring file locations, or need deeper,
//! tool-specific parsing than is worth building until a real migration
//! needs it.

use std::{fs, path::Path};

/// A release-tooling config file found in the project, with any file paths
/// it declared as holding the version (empty when only presence was
/// detected, not parsed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseToolingHit {
    pub tool: &'static str,
    pub config_path: String,
    pub imported_version_files: Vec<String>,
}

const PRESENCE_ONLY_CANDIDATES: &[(&str, &str)] = &[
    (".releaserc", "semantic-release"),
    (".releaserc.json", "semantic-release"),
    (".releaserc.yml", "semantic-release"),
    (".releaserc.yaml", "semantic-release"),
    ("release.config.js", "semantic-release"),
    ("release.config.cjs", "semantic-release"),
    ("release-please-config.json", "release-please"),
    (".release-please-manifest.json", "release-please"),
    (".changeset/config.json", "changesets"),
    ("GitVersion.yml", "gitversion"),
    ("GitVersion.yaml", "gitversion"),
    ("gitversion.yml", "gitversion"),
    ("version.json", "nerdbank-gitversioning"),
    ("release.toml", "cargo-release"),
    (".goreleaser.yml", "goreleaser"),
    (".goreleaser.yaml", "goreleaser"),
    ("lerna.json", "lerna"),
];

/// Scans `root` (top level only, plus `.changeset/` for changesets) for
/// known release-tooling configs.
pub fn detect(root: &Path) -> Vec<ReleaseToolingHit> {
    let mut hits = Vec::new();

    for (name, tool) in PRESENCE_ONLY_CANDIDATES {
        if root.join(name).is_file() {
            hits.push(ReleaseToolingHit {
                tool,
                config_path: name.to_string(),
                imported_version_files: Vec::new(),
            });
        }
    }

    if let Some(files) = import_bumpversion_cfg(root) {
        hits.push(ReleaseToolingHit {
            tool: "bumpversion",
            config_path: ".bumpversion.cfg".to_string(),
            imported_version_files: files,
        });
    }

    if let Some(files) = import_tbump_toml(root) {
        hits.push(ReleaseToolingHit {
            tool: "tbump",
            config_path: "tbump.toml".to_string(),
            imported_version_files: files,
        });
    }

    hits
}

/// Parses `.bumpversion.cfg`'s classic INI format: each
/// `[bumpversion:file:PATH]` (or `[bumpversion:file (glob):PATTERN]`, which
/// is left unresolved since it isn't a literal path) section names a file
/// that carries the version.
fn import_bumpversion_cfg(root: &Path) -> Option<Vec<String>> {
    let content = fs::read_to_string(root.join(".bumpversion.cfg")).ok()?;
    let mut files = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
            continue;
        }
        let section = &trimmed[1..trimmed.len() - 1];
        if let Some(path) = section.strip_prefix("bumpversion:file:") {
            if !path.is_empty() {
                files.push(path.to_string());
            }
        }
    }

    Some(files)
}

/// Parses `tbump.toml`'s `[[file]]` array of tables, each with a `src`
/// field naming a file that carries the version.
fn import_tbump_toml(root: &Path) -> Option<Vec<String>> {
    let content = fs::read_to_string(root.join("tbump.toml")).ok()?;
    let parsed: toml::Value = content.parse().ok()?;

    let files = parsed.get("file")?.as_array()?;
    let paths = files
        .iter()
        .filter_map(|entry| entry.get("src")?.as_str().map(str::to_string))
        .collect();
    Some(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "mvs-importers-test-{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn imports_bumpversion_cfg_file_sections() {
        let dir = TempDir::new("bumpversion");
        fs::write(
            dir.path().join(".bumpversion.cfg"),
            "[bumpversion]\ncurrent_version = 1.2.3\n\n\
             [bumpversion:file:setup.py]\n\n\
             [bumpversion:file:src/pkg/__init__.py]\n\n\
             [bumpversion:file (glob):**/version.txt]\n",
        )
        .unwrap();

        let hits = detect(dir.path());
        let hit = hits
            .iter()
            .find(|h| h.tool == "bumpversion")
            .expect("bumpversion detected");
        assert_eq!(
            hit.imported_version_files,
            vec!["setup.py".to_string(), "src/pkg/__init__.py".to_string()]
        );
    }

    #[test]
    fn imports_tbump_toml_file_entries() {
        let dir = TempDir::new("tbump");
        fs::write(
            dir.path().join("tbump.toml"),
            r#"
[version]
current = "1.2.3"

[[file]]
src = "pyproject.toml"

[[file]]
src = "src/pkg/__init__.py"
"#,
        )
        .unwrap();

        let hits = detect(dir.path());
        let hit = hits
            .iter()
            .find(|h| h.tool == "tbump")
            .expect("tbump detected");
        assert_eq!(
            hit.imported_version_files,
            vec![
                "pyproject.toml".to_string(),
                "src/pkg/__init__.py".to_string()
            ]
        );
    }

    #[test]
    fn presence_only_tools_are_reported_without_file_lists() {
        let dir = TempDir::new("presence");
        fs::write(dir.path().join("lerna.json"), "{}").unwrap();
        fs::write(dir.path().join(".goreleaser.yml"), "builds: []").unwrap();

        let hits = detect(dir.path());
        assert!(hits
            .iter()
            .any(|h| h.tool == "lerna" && h.imported_version_files.is_empty()));
        assert!(hits
            .iter()
            .any(|h| h.tool == "goreleaser" && h.imported_version_files.is_empty()));
    }

    #[test]
    fn nothing_detected_in_a_bare_directory() {
        let dir = TempDir::new("bare");
        assert!(detect(dir.path()).is_empty());
    }
}
