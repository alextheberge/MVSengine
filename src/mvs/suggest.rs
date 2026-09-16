// SPDX-License-Identifier: AGPL-3.0-only
//! Proposes `@mvs-protocol`/`@mvs-feature` decorators for undecorated public
//! API surface, and inserts them using the same comment tokenization the
//! crawler reads back — so a written suggestion is guaranteed to be picked
//! up by the very next `lint`/`generate`.
//!
//! `@mvs-protocol`/`@mvs-feature` tags are scoped to the whole file they
//! appear in (the crawler collects every comment in a file, not just ones
//! attached to a specific declaration), so placement only has to be
//! *somewhere sensible* in the file, not exactly above one symbol. This
//! module places suggestions right after any leading header comment/shebang,
//! before the first real line of code.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::{Context, Result};

use crate::mvs::crawler::crawl_codebase;
use crate::mvs::manifest::ScanPolicy;
use crate::mvs::vcs;

const COMMIT_SCOPE_LOOKBACK: usize = 300;
const MAX_COMMIT_SCOPES_SURFACED: usize = 20;
const GENERIC_FILE_STEMS: &[&str] = &["mod", "index", "__init__", "main", "lib"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionKind {
    /// One per undecorated file that has public API surface: a boundary.
    Protocol,
    /// One per undecorated directory of files with public API surface: a
    /// capability cluster.
    Feature,
}

impl SuggestionKind {
    fn tag_name(self) -> &'static str {
        match self {
            SuggestionKind::Protocol => "mvs-protocol",
            SuggestionKind::Feature => "mvs-feature",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProtocolSuggestion {
    pub name: String,
    pub file: String,
    pub item_count: usize,
}

#[derive(Debug, Clone)]
pub struct FeatureSuggestion {
    pub name: String,
    pub directory: String,
    pub target_file: String,
    pub file_count: usize,
}

#[derive(Debug, Default)]
pub struct SuggestionSet {
    pub protocols: Vec<ProtocolSuggestion>,
    pub features: Vec<FeatureSuggestion>,
    /// The most common Conventional Commits scopes seen in recent history
    /// (`feat(scope): ...`), surfaced for visibility and used to prefer a
    /// team's own naming over a guessed one when they match.
    pub commit_scopes_considered: Vec<String>,
}

/// Crawls `root` under `scan_policy` and proposes decorators for every file
/// (protocol) / directory (feature) that has public API surface but no
/// existing tag of that kind anywhere in it. Already-decorated
/// files/directories are never suggested again, so re-running after writing
/// suggestions naturally finds nothing left to propose.
pub fn build_suggestions(root: &Path, scan_policy: &ScanPolicy) -> Result<SuggestionSet> {
    let crawl = crawl_codebase(root, scan_policy)
        .with_context(|| format!("failed to crawl source root: {}", root.display()))?;

    let decorated_protocol_files: BTreeSet<&str> = crawl
        .protocol_occurrences
        .iter()
        .map(|occurrence| occurrence.file.as_str())
        .collect();
    let decorated_feature_dirs: BTreeSet<String> = crawl
        .feature_occurrences
        .iter()
        .map(|occurrence| capability_dir(&occurrence.file))
        .collect();

    let mut items_by_file: BTreeMap<&str, usize> = BTreeMap::new();
    for item in &crawl.public_api {
        *items_by_file.entry(item.file.as_str()).or_default() += 1;
    }

    let mut protocols: Vec<ProtocolSuggestion> = items_by_file
        .iter()
        .filter(|(file, _)| !decorated_protocol_files.contains(*file))
        .map(|(file, count)| ProtocolSuggestion {
            name: derive_name_from_file(file),
            file: (*file).to_string(),
            item_count: *count,
        })
        .collect();
    dedupe_names(protocols.iter_mut().map(|suggestion| &mut suggestion.name));
    protocols.sort_by(|a, b| a.file.cmp(&b.file));

    let mut files_by_dir: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for file in items_by_file.keys() {
        files_by_dir
            .entry(capability_dir(file))
            .or_default()
            .push(file);
    }

    let commit_scopes = if vcs::is_available() && vcs::is_repo(root) {
        vcs::list_commit_scopes(root, COMMIT_SCOPE_LOOKBACK).unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut features: Vec<FeatureSuggestion> = files_by_dir
        .iter()
        .filter(|(dir, _)| !decorated_feature_dirs.contains(*dir))
        .map(|(dir, files)| {
            let mut sorted_files = files.clone();
            sorted_files.sort_unstable();
            let raw_name = derive_name_from_dir(dir);
            FeatureSuggestion {
                name: prefer_commit_scope(&raw_name, &commit_scopes),
                directory: dir.clone(),
                target_file: sorted_files[0].to_string(),
                file_count: files.len(),
            }
        })
        .collect();
    dedupe_names(features.iter_mut().map(|suggestion| &mut suggestion.name));
    features.sort_by(|a, b| a.directory.cmp(&b.directory));

    Ok(SuggestionSet {
        protocols,
        features,
        commit_scopes_considered: commit_scopes
            .into_iter()
            .take(MAX_COMMIT_SCOPES_SURFACED)
            .map(|(scope, _)| scope)
            .collect(),
    })
}

/// Inserts a protocol or feature decorator comment carrying `name` into
/// `root`/`relative_file`, using that file's comment syntax. Returns `Ok(true)`
/// when written, `Ok(false)` when the file's language isn't one `--write`
/// supports (the suggestion should still be reported, just not applied).
pub fn insert_decorator_comment(
    root: &Path,
    relative_file: &str,
    kind: SuggestionKind,
    name: &str,
) -> Result<bool> {
    let path = root.join(relative_file);
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read `{}`", path.display()))?;
    let style = comment_style_for(relative_file);

    let updated = match style {
        CommentStyle::Unsupported => return Ok(false),
        CommentStyle::Php => {
            let Some(open_tag_line) = find_php_open_tag_line(&content) else {
                return Ok(false);
            };
            let lines: Vec<&str> = content.lines().collect();
            splice_line_after(
                &lines,
                open_tag_line,
                &decorator_comment_line(style, kind, name),
            )
        }
        _ => {
            let lines: Vec<&str> = content.lines().collect();
            let insert_at = find_insertion_line(&lines, comment_leader_for_skip(style));
            splice_line_before(
                &lines,
                insert_at,
                &decorator_comment_line(style, kind, name),
            )
        }
    };

    fs::write(&path, updated).with_context(|| format!("failed to write `{}`", path.display()))?;
    Ok(true)
}

// ── naming ────────────────────────────────────────────────────────────────

fn capability_dir(file: &str) -> String {
    match Path::new(file).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            parent.to_string_lossy().replace('\\', "/")
        }
        _ => String::new(),
    }
}

fn derive_name_from_dir(dir: &str) -> String {
    let last = Path::new(dir)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("root");
    to_snake_case(last)
}

fn derive_name_from_file(file: &str) -> String {
    let path = Path::new(file);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");
    let parent_name = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str());

    let raw = if GENERIC_FILE_STEMS.contains(&stem) {
        parent_name.unwrap_or(stem).to_string()
    } else {
        match parent_name {
            Some(parent) if parent != "src" => format!("{parent}_{stem}"),
            _ => stem.to_string(),
        }
    };
    to_snake_case(&raw)
}

fn to_snake_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_was_sep = true; // avoid a leading underscore
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep {
            out.push('_');
            last_was_sep = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "module".to_string()
    } else {
        out
    }
}

/// Prefers a Conventional Commits scope's own spelling/casing over a
/// guessed name when they're the same word, so `feat(offline-storage): ...`
/// in history yields `offline-storage`, not a re-derived `offline_storage`.
fn prefer_commit_scope(raw_name: &str, scopes: &[(String, usize)]) -> String {
    let normalized_raw = to_snake_case(raw_name);
    scopes
        .iter()
        .find(|(scope, _)| to_snake_case(scope) == normalized_raw)
        .map(|(scope, _)| scope.clone())
        .unwrap_or_else(|| raw_name.to_string())
}

fn dedupe_names<'a>(names: impl Iterator<Item = &'a mut String>) {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for name in names {
        let count = seen.entry(name.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            *name = format!("{name}_{count}");
        }
    }
}

// ── insertion ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommentStyle {
    /// `///` (this codebase's, and Rust's doc-comment convention).
    Rust,
    /// `//` (TS/JS/Go/Java/Kotlin/C#/Swift/Dart).
    CStyle,
    /// `#` (Python/Ruby).
    Hash,
    /// `--` (Lua/Luau).
    DoubleDash,
    /// `.php` files are HTML/text outside `<?php ... ?>`, so a comment can
    /// only go after the opening tag, never at the top of the file.
    Php,
    Unsupported,
}

fn comment_style_for(file: &str) -> CommentStyle {
    match Path::new(file).extension().and_then(|ext| ext.to_str()) {
        Some("rs") => CommentStyle::Rust,
        Some("ts" | "tsx" | "js" | "jsx" | "go" | "java" | "kt" | "cs" | "swift" | "dart") => {
            CommentStyle::CStyle
        }
        Some("py" | "rb") => CommentStyle::Hash,
        Some("php") => CommentStyle::Php,
        Some("lua" | "luau") => CommentStyle::DoubleDash,
        _ => CommentStyle::Unsupported,
    }
}

fn comment_leader_for_skip(style: CommentStyle) -> &'static str {
    match style {
        CommentStyle::Rust | CommentStyle::CStyle => "//",
        CommentStyle::Hash => "#",
        CommentStyle::DoubleDash => "--",
        CommentStyle::Php | CommentStyle::Unsupported => "",
    }
}

fn decorator_comment_line(style: CommentStyle, kind: SuggestionKind, name: &str) -> String {
    let tag = kind.tag_name();
    match style {
        CommentStyle::Rust => format!("/// @{tag}(\"{name}\")"),
        CommentStyle::Hash => format!("# @{tag}(\"{name}\")"),
        CommentStyle::DoubleDash => format!("-- @{tag}(\"{name}\")"),
        CommentStyle::CStyle | CommentStyle::Php => format!("// @{tag}(\"{name}\")"),
        CommentStyle::Unsupported => unreachable!("caller returns before formatting"),
    }
}

/// Skips any leading blank lines, then a contiguous run of `leader`-prefixed
/// comment lines right after them (a license header, a shebang, ...),
/// stopping at the first line that is neither — which may itself be blank
/// (in which case the decorator is inserted right after the header, not
/// after the gap that follows it).
fn find_insertion_line(lines: &[&str], leader: &str) -> usize {
    let mut i = 0;
    while i < lines.len() && lines[i].trim().is_empty() {
        i += 1;
    }
    while i < lines.len() && !leader.is_empty() && lines[i].trim_start().starts_with(leader) {
        i += 1;
    }
    i
}

fn find_php_open_tag_line(content: &str) -> Option<usize> {
    content
        .lines()
        .position(|line| line.contains("<?php") || line.trim_start().starts_with("<?="))
}

fn splice_line_before(lines: &[&str], index: usize, new_line: &str) -> String {
    let mut out_lines: Vec<&str> = Vec::with_capacity(lines.len() + 2);
    out_lines.extend_from_slice(&lines[..index]);
    out_lines.push(new_line);
    if index < lines.len() && !lines[index].trim().is_empty() {
        out_lines.push("");
    }
    out_lines.extend_from_slice(&lines[index..]);
    let mut result = out_lines.join("\n");
    result.push('\n');
    result
}

fn splice_line_after(lines: &[&str], index: usize, new_line: &str) -> String {
    let mut out_lines: Vec<&str> = Vec::with_capacity(lines.len() + 1);
    out_lines.extend_from_slice(&lines[..=index]);
    out_lines.push(new_line);
    out_lines.extend_from_slice(&lines[index + 1..]);
    let mut result = out_lines.join("\n");
    result.push('\n');
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "mvs-suggest-test-{label}-{}-{}",
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
    fn to_snake_case_normalizes_separators_and_case() {
        assert_eq!(to_snake_case("OfflineStorage"), "offlinestorage");
        assert_eq!(to_snake_case("offline-storage"), "offline_storage");
        assert_eq!(to_snake_case("  weird__Name--"), "weird_name");
        assert_eq!(to_snake_case("---"), "module");
    }

    #[test]
    fn derive_name_from_file_prefers_parent_for_generic_stems() {
        assert_eq!(derive_name_from_file("src/commands/mod.rs"), "commands");
        assert_eq!(
            derive_name_from_file("src/commands/migrate.rs"),
            "commands_migrate"
        );
        assert_eq!(derive_name_from_file("src/lib.rs"), "src");
        assert_eq!(derive_name_from_file("lib.rs"), "lib");
    }

    #[test]
    fn capability_dir_is_the_immediate_parent() {
        assert_eq!(capability_dir("src/commands/migrate.rs"), "src/commands");
        assert_eq!(capability_dir("lib.rs"), "");
    }

    #[test]
    fn prefer_commit_scope_matches_case_and_separator_insensitively() {
        let scopes = vec![("offline-storage".to_string(), 5), ("auth".to_string(), 2)];
        assert_eq!(
            prefer_commit_scope("offline_storage", &scopes),
            "offline-storage"
        );
        assert_eq!(prefer_commit_scope("billing", &scopes), "billing");
    }

    #[test]
    fn dedupe_names_appends_numeric_suffixes_on_collision() {
        let mut names = vec!["auth".to_string(), "auth".to_string(), "auth".to_string()];
        dedupe_names(names.iter_mut());
        assert_eq!(names, vec!["auth", "auth_2", "auth_3"]);
    }

    #[test]
    fn build_suggestions_groups_by_file_and_directory_skipping_decorated_ones() {
        let dir = TempDir::new("build");
        fs::create_dir_all(dir.path().join("src/auth")).unwrap();
        fs::create_dir_all(dir.path().join("src/billing")).unwrap();

        // src/auth: two files, no existing tags -> one feature + two protocol suggestions.
        fs::write(dir.path().join("src/auth/login.rs"), "pub fn login() {}\n").unwrap();
        fs::write(
            dir.path().join("src/auth/logout.rs"),
            "pub fn logout() {}\n",
        )
        .unwrap();

        // src/billing: already has a feature tag -> no feature suggestion, but the
        // undecorated file still gets a protocol suggestion.
        fs::write(
            dir.path().join("src/billing/invoice.rs"),
            "//! @mvs-feature(\"billing\")\npub fn charge() {}\n",
        )
        .unwrap();

        let scan_policy = ScanPolicy {
            rust_export_following: crate::mvs::manifest::RustExportFollowing::PublicModules,
            ..ScanPolicy::default()
        };

        let suggestions = build_suggestions(dir.path(), &scan_policy).unwrap();

        assert_eq!(suggestions.protocols.len(), 3);
        assert!(suggestions
            .protocols
            .iter()
            .any(|p| p.file == "src/auth/login.rs" && p.name == "auth_login"));
        assert!(suggestions
            .protocols
            .iter()
            .any(|p| p.file == "src/billing/invoice.rs"));

        assert_eq!(suggestions.features.len(), 1);
        assert_eq!(suggestions.features[0].directory, "src/auth");
        assert_eq!(suggestions.features[0].name, "auth");
        assert_eq!(suggestions.features[0].file_count, 2);
        assert_eq!(suggestions.features[0].target_file, "src/auth/login.rs");
    }

    #[test]
    fn insert_decorator_comment_skips_rust_header_and_uses_triple_slash() {
        let dir = TempDir::new("rust-insert");
        let content =
            "// SPDX-License-Identifier: AGPL-3.0-only\nuse std::fmt;\n\npub fn hello() {}\n";
        fs::write(dir.path().join("lib.rs"), content).unwrap();

        let written =
            insert_decorator_comment(dir.path(), "lib.rs", SuggestionKind::Protocol, "core")
                .unwrap();
        assert!(written);

        let updated = fs::read_to_string(dir.path().join("lib.rs")).unwrap();
        let lines: Vec<&str> = updated.lines().collect();
        assert_eq!(lines[0], "// SPDX-License-Identifier: AGPL-3.0-only");
        assert_eq!(lines[1], "/// @mvs-protocol(\"core\")");
        assert!(updated.contains("use std::fmt;"));
        assert!(updated.contains("pub fn hello() {}"));
    }

    #[test]
    fn insert_decorator_comment_skips_python_shebang() {
        let dir = TempDir::new("python-insert");
        fs::write(
            dir.path().join("app.py"),
            "#!/usr/bin/env python3\n\ndef hello():\n    pass\n",
        )
        .unwrap();

        insert_decorator_comment(dir.path(), "app.py", SuggestionKind::Feature, "web").unwrap();

        let updated = fs::read_to_string(dir.path().join("app.py")).unwrap();
        let lines: Vec<&str> = updated.lines().collect();
        assert_eq!(lines[0], "#!/usr/bin/env python3");
        assert_eq!(lines[1], "# @mvs-feature(\"web\")");
    }

    #[test]
    fn insert_decorator_comment_goes_after_php_open_tag() {
        let dir = TempDir::new("php-insert");
        fs::write(
            dir.path().join("index.php"),
            "<html>\n<body>\n<?php\nfunction hello() {}\n",
        )
        .unwrap();

        insert_decorator_comment(dir.path(), "index.php", SuggestionKind::Protocol, "web").unwrap();

        let updated = fs::read_to_string(dir.path().join("index.php")).unwrap();
        let lines: Vec<&str> = updated.lines().collect();
        assert_eq!(lines[2], "<?php");
        assert_eq!(lines[3], "// @mvs-protocol(\"web\")");
    }

    #[test]
    fn insert_decorator_comment_reports_unsupported_extensions_without_writing() {
        let dir = TempDir::new("unsupported-insert");
        fs::write(dir.path().join("notes.md"), "# hi\n").unwrap();

        let written =
            insert_decorator_comment(dir.path(), "notes.md", SuggestionKind::Protocol, "x")
                .unwrap();
        assert!(!written);
        assert_eq!(
            fs::read_to_string(dir.path().join("notes.md")).unwrap(),
            "# hi\n"
        );
    }
}
