// SPDX-License-Identifier: AGPL-3.0-only
//! Reads and writes the version string declared in common ecosystem manifest
//! files (`Cargo.toml`, `package.json`, `pyproject.toml`, ...), so `mvs-manager
//! sync` can keep every registry-facing file in lock-step with the MVS
//! identity's SemVer projection (`arch.feat.fix`).
//!
//! Every adapter here edits only the byte span holding the version value.
//! Everything else in the file — formatting, key order, comments, unrelated
//! fields — is left untouched.

use std::{fs, ops::Range, path::Path};

use anyhow::{anyhow, Context, Result};
use regex::Regex;

use crate::mvs::manifest::{VersionFileEntry, VersionFileKind, VersionProjection};

const SIMPLE_DETECTION_CANDIDATES: &[(&str, VersionFileKind)] = &[
    ("Cargo.toml", VersionFileKind::CargoToml),
    ("package.json", VersionFileKind::NpmPackageJson),
    ("composer.json", VersionFileKind::ComposerJson),
    ("pubspec.yaml", VersionFileKind::PubspecYaml),
    ("gradle.properties", VersionFileKind::GradleProperties),
    ("build.gradle", VersionFileKind::BuildGradle),
    ("build.gradle.kts", VersionFileKind::BuildGradle),
    ("pom.xml", VersionFileKind::PomXml),
];

const GLOB_DETECTION_CANDIDATES: &[(&str, VersionFileKind)] = &[
    ("csproj", VersionFileKind::Csproj),
    ("gemspec", VersionFileKind::GemspecLiteral),
    ("rockspec", VersionFileKind::LuaRockspec),
];

/// Scans `root` (top level only) for files whose version this crate knows how
/// to read and write. Only files where a version field can actually be
/// located are returned, so a workspace-inherited or fully dynamic version
/// (e.g. `version.workspace = true`, `dynamic = ["version"]`) is silently
/// skipped rather than reported as a false positive.
pub fn detect(root: &Path) -> Vec<VersionFileEntry> {
    let mut found = Vec::new();

    for (name, kind) in SIMPLE_DETECTION_CANDIDATES {
        if let Some(entry) = probe(root, name, *kind) {
            found.push(entry);
        }
    }

    if let Ok(content) = fs::read_to_string(root.join("pyproject.toml")) {
        if locate_scoped_toml_version(&content, "tool.poetry").is_some() {
            found.push(make_entry(
                "pyproject.toml",
                VersionFileKind::PyprojectPoetry,
            ));
        } else if locate_scoped_toml_version(&content, "project").is_some() {
            found.push(make_entry(
                "pyproject.toml",
                VersionFileKind::PyprojectPep621,
            ));
        }
    }

    for (ext, kind) in GLOB_DETECTION_CANDIDATES {
        if let Some(name) = first_file_with_extension(root, ext) {
            if let Some(entry) = probe(root, &name, *kind) {
                found.push(entry);
            }
        }
    }

    for candidate in ["VERSION", "VERSION.txt"] {
        if let Ok(content) = fs::read_to_string(root.join(candidate)) {
            if !content.trim().is_empty() {
                found.push(make_entry(candidate, VersionFileKind::PlainVersionFile));
                break;
            }
        }
    }

    found
}

/// Reads the currently declared version out of `entry`, relative to `root`.
pub fn read_version(root: &Path, entry: &VersionFileEntry) -> Result<String> {
    let path = root.join(&entry.path);
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read version file `{}`", path.display()))?;

    if entry.kind == VersionFileKind::PlainVersionFile {
        return Ok(content.trim().to_string());
    }

    let span = locate_version_span(entry.kind, &content)
        .ok_or_else(|| version_not_found_error(&path, entry.kind))?;
    Ok(content[span].to_string())
}

/// Writes `new_version` into `entry`'s declared version span, relative to
/// `root`. Returns `Ok(true)` when the file changed, `Ok(false)` when it
/// already matched.
pub fn write_version(root: &Path, entry: &VersionFileEntry, new_version: &str) -> Result<bool> {
    let path = root.join(&entry.path);
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read version file `{}`", path.display()))?;

    let updated = if entry.kind == VersionFileKind::PlainVersionFile {
        if content.trim() == new_version {
            return Ok(false);
        }
        if content.ends_with('\n') {
            format!("{new_version}\n")
        } else {
            new_version.to_string()
        }
    } else {
        let span = locate_version_span(entry.kind, &content)
            .ok_or_else(|| version_not_found_error(&path, entry.kind))?;
        if &content[span.clone()] == new_version {
            return Ok(false);
        }
        let mut buf = String::with_capacity(content.len() + new_version.len());
        buf.push_str(&content[..span.start]);
        buf.push_str(new_version);
        buf.push_str(&content[span.end..]);
        buf
    };

    fs::write(&path, updated)
        .with_context(|| format!("failed to write version file `{}`", path.display()))?;
    Ok(true)
}

/// Convenience for computing what should be written into `entry` from a
/// manifest identity via its declared (or default) projection.
pub fn projected_version(
    entry: &VersionFileEntry,
    identity: &crate::mvs::manifest::Identity,
) -> String {
    entry.projection.project(identity)
}

fn version_not_found_error(path: &Path, kind: VersionFileKind) -> anyhow::Error {
    anyhow!(
        "could not locate a version field in `{}` (kind: {}); it may be workspace-inherited, \
         dynamically computed, or in an unexpected shape",
        path.display(),
        kind.as_str()
    )
}

fn probe(root: &Path, name: &str, kind: VersionFileKind) -> Option<VersionFileEntry> {
    let content = fs::read_to_string(root.join(name)).ok()?;
    locate_version_span(kind, &content)?;
    Some(make_entry(name, kind))
}

fn make_entry(path: &str, kind: VersionFileKind) -> VersionFileEntry {
    VersionFileEntry {
        path: path.to_string(),
        kind,
        projection: VersionProjection::default(),
    }
}

fn first_file_with_extension(root: &Path, ext: &str) -> Option<String> {
    let entries = fs::read_dir(root).ok()?;
    let mut matches: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some(ext) {
                path.file_name().map(|n| n.to_string_lossy().to_string())
            } else {
                None
            }
        })
        .collect();
    matches.sort();
    matches.into_iter().next()
}

// ── Span location, dispatched per kind ───────────────────────────────────────

fn locate_version_span(kind: VersionFileKind, content: &str) -> Option<Range<usize>> {
    match kind {
        VersionFileKind::CargoToml => locate_scoped_toml_version(content, "package")
            .or_else(|| locate_scoped_toml_version(content, "workspace.package")),
        VersionFileKind::PyprojectPep621 => locate_scoped_toml_version(content, "project"),
        VersionFileKind::PyprojectPoetry => locate_scoped_toml_version(content, "tool.poetry"),
        VersionFileKind::NpmPackageJson | VersionFileKind::ComposerJson => {
            locate_json_top_level_string_field(content, "version")
        }
        VersionFileKind::PubspecYaml => locate_top_level_line_value(content, "version:")
            .map(|span| narrow_before_char(content, span, '+')),
        VersionFileKind::GradleProperties => locate_top_level_line_value(content, "version="),
        VersionFileKind::BuildGradle => {
            locate_first_regex_group(content, r#"(?m)^\s*version\s*=\s*["']([^"']+)["']"#).or_else(
                || locate_first_regex_group(content, r#"(?m)^\s*version\s+["']([^"']+)["']"#),
            )
        }
        VersionFileKind::PomXml => locate_pom_version(content),
        VersionFileKind::Csproj => locate_first_element_text(content, "Version")
            .or_else(|| locate_first_element_text(content, "VersionPrefix")),
        VersionFileKind::GemspecLiteral => {
            locate_first_regex_group(content, r#"\.version\s*=\s*["']([^"']+)["']"#)
        }
        VersionFileKind::RubyVersionConstant => {
            locate_first_regex_group(content, r#"VERSION\s*=\s*["']([^"']+)["']"#)
        }
        VersionFileKind::LuaRockspec => {
            locate_first_regex_group(content, r#"(?m)^\s*version\s*=\s*["']([^"']+)["']"#)
                .map(|span| narrow_before_rockspec_revision(content, span))
        }
        VersionFileKind::PlainVersionFile => None,
    }
}

/// Locates `version = "…"` inside a specific dotted TOML table (`[section]`),
/// matched by an exact `[section]` header line (not sub-tables or arrays of
/// tables). This keeps `Cargo.toml`'s `[dependencies]` versions and
/// `pyproject.toml`'s `[tool.poetry.dependencies]` versions out of scope.
fn locate_scoped_toml_version(content: &str, section: &str) -> Option<Range<usize>> {
    let target = format!("[{section}]");
    let version_re = toml_version_line_regex();
    let mut offset = 0usize;
    let mut in_section = false;

    for line in content.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed == target;
            offset += line.len();
            continue;
        }
        if in_section {
            // Rust's `regex` crate, unlike e.g. Python's `re`, does not treat
            // `$` as matching just before a trailing `\n` — it only matches
            // the true end of the haystack. `line` (from `split_inclusive`)
            // always ends with `\n`, so a `$`-anchored pattern would fail to
            // match any line with a trailing `# comment` (the `.` in `#.*`
            // can't consume the `\n`, and `$` then has nothing left to match
            // against). Stripping the line terminator before matching keeps
            // the anchor meaning what it looks like it means; byte offsets
            // are unaffected since we only trim off the end.
            let line_without_terminator = line.trim_end_matches(['\n', '\r']);
            if let Some(caps) = version_re.captures(line_without_terminator) {
                let m = caps.get(1).expect("capture group 1 present");
                return Some((offset + m.start())..(offset + m.end()));
            }
        }
        offset += line.len();
    }
    None
}

fn toml_version_line_regex() -> Regex {
    Regex::new(r#"^\s*version\s*=\s*["']([^"']*)["']\s*(?:#.*)?$"#).expect("valid regex")
}

/// Locates a top-level (column-zero) `prefix<value>` line, such as YAML's
/// `version: 1.2.3` or a `.properties` file's `version=1.2.3`. Trailing
/// comments (`#...`) and CR/LF are excluded from the returned span.
fn locate_top_level_line_value(content: &str, prefix: &str) -> Option<Range<usize>> {
    let mut offset = 0usize;
    for line in content.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix(prefix) {
            let after_ws = rest.len() - rest.trim_start().len();
            let trimmed = rest.trim_start();

            let mut end_in_trimmed = trimmed.len();
            for marker in ['\r', '\n', '#'] {
                if let Some(pos) = trimmed.find(marker) {
                    end_in_trimmed = end_in_trimmed.min(pos);
                }
            }
            let value = trimmed[..end_in_trimmed].trim_end();
            if !value.is_empty() {
                let value_start = prefix.len() + after_ws;
                let value_end = value_start + value.len();
                return Some((offset + value_start)..(offset + value_end));
            }
        }
        offset += line.len();
    }
    None
}

fn locate_first_regex_group(content: &str, pattern: &str) -> Option<Range<usize>> {
    let re = Regex::new(pattern).expect("valid regex");
    let caps = re.captures(content)?;
    let m = caps.get(1)?;
    Some(m.start()..m.end())
}

/// Locates the text content of the first `<element>...</element>`, anywhere
/// in the document. Suitable for simple, unambiguous files such as `.csproj`
/// where `Version` never legitimately repeats in a meaningfully different
/// scope.
fn locate_first_element_text(content: &str, element: &str) -> Option<Range<usize>> {
    let open = format!("<{element}>");
    let close = format!("</{element}>");
    let start_tag = content.find(&open)?;
    let text_start = start_tag + open.len();
    let close_rel = content[text_start..].find(&close)?;
    let text_end = text_start + close_rel;
    let raw = &content[text_start..text_end];
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let leading_ws = raw.len() - raw.trim_start().len();
    let value_start = text_start + leading_ws;
    Some(value_start..(value_start + trimmed.len()))
}

/// Locates the project's own `<version>` in a Maven `pom.xml`: the first
/// `<version>` element that is not nested inside `<parent>`,
/// `<dependencies>`, `<dependencyManagement>`, `<build>`, `<profiles>`,
/// `<pluginManagement>`, or `<reporting>`. This is a lightweight tag scanner,
/// not a full XML parser, but it correctly ignores parent/dependency
/// versions that are not the project's own.
fn locate_pom_version(content: &str) -> Option<Range<usize>> {
    const SKIP_ELEMENTS: &[&str] = &[
        "parent",
        "dependencies",
        "dependencyManagement",
        "build",
        "profiles",
        "pluginManagement",
        "reporting",
    ];

    let mut skip_stack: Vec<String> = Vec::new();
    let mut i = 0usize;

    loop {
        let lt_rel = content[i..].find('<')?;
        let tag_start = i + lt_rel;
        let gt_rel = content[tag_start..].find('>')?;
        let tag_end = tag_start + gt_rel;
        let raw_tag = &content[tag_start + 1..tag_end];
        i = tag_end + 1;

        if raw_tag.starts_with('!') || raw_tag.starts_with('?') {
            continue;
        }

        let is_closing = raw_tag.starts_with('/');
        let is_self_closing = raw_tag.trim_end().ends_with('/');
        let name = raw_tag
            .trim_start_matches('/')
            .trim_end_matches('/')
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();

        if is_closing {
            if skip_stack.last().map(|s| s.as_str()) == Some(name.as_str()) {
                skip_stack.pop();
            }
            continue;
        }

        if !skip_stack.is_empty() {
            if !is_self_closing && SKIP_ELEMENTS.contains(&name.as_str()) {
                skip_stack.push(name);
            }
            continue;
        }

        if SKIP_ELEMENTS.contains(&name.as_str()) {
            if !is_self_closing {
                skip_stack.push(name);
            }
            continue;
        }

        if name == "version" && !is_self_closing {
            let text_start = tag_end + 1;
            let close_tag_rel = content[text_start..].find("</version>")?;
            let text_end = text_start + close_tag_rel;
            let raw = &content[text_start..text_end];
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            let leading_ws = raw.len() - raw.trim_start().len();
            let value_start = text_start + leading_ws;
            return Some(value_start..(value_start + trimmed.len()));
        }
    }
}

/// Locates the `"field"` value of a top-level (depth-1) key in a JSON
/// document, ignoring same-named keys nested inside other objects or arrays.
fn locate_json_top_level_string_field(content: &str, field: &str) -> Option<Range<usize>> {
    let bytes = content.as_bytes();
    let mut i = 0usize;
    let mut depth: i32 = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                i += 1;
            }
            b'"' => {
                let (key, after) = read_json_string(content, i)?;
                if depth == 1 && key == field {
                    let mut j = skip_json_whitespace(bytes, after);
                    if j < bytes.len() && bytes[j] == b':' {
                        j = skip_json_whitespace(bytes, j + 1);
                        if j < bytes.len() && bytes[j] == b'"' {
                            let (_, value_after) = read_json_string(content, j)?;
                            return Some((j + 1)..(value_after - 1));
                        }
                    }
                }
                i = after;
            }
            _ => i += 1,
        }
    }
    None
}

fn skip_json_whitespace(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Reads a JSON string literal starting at byte index `start` (must point at
/// the opening `"`). The returned text is only used for equality checks
/// against plain-ASCII field names, so escape sequences are copied through
/// verbatim rather than fully decoded; the returned end index (just past the
/// closing quote) is exact.
fn read_json_string(content: &str, start: usize) -> Option<(String, usize)> {
    let bytes = content.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return None;
    }
    let mut i = start + 1;
    let mut out = String::new();
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                if i + 1 >= bytes.len() {
                    return None;
                }
                out.push('\\');
                i += 2;
            }
            b'"' => return Some((out, i + 1)),
            b => {
                // Byte-wise reconstruction: fine for equality checks against
                // ASCII field names, and the byte offsets used by callers
                // never depend on `out` being a faithful decode.
                out.push(b as char);
                i += 1;
            }
        }
    }
    None
}

fn narrow_before_char(content: &str, span: Range<usize>, ch: char) -> Range<usize> {
    let text = &content[span.clone()];
    match text.find(ch) {
        Some(pos) => span.start..(span.start + pos),
        None => span,
    }
}

/// Preserves a LuaRocks revision suffix (`-N`) so a rewrite of `1.2.0-1`
/// only touches the `1.2.0` portion.
fn narrow_before_rockspec_revision(content: &str, span: Range<usize>) -> Range<usize> {
    let text = &content[span.clone()];
    if let Some(dash_pos) = text.rfind('-') {
        let digits = &text[dash_pos + 1..];
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            return span.start..(span.start + dash_pos);
        }
    }
    span
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvs::manifest::Identity;
    use std::fs;
    use tempfile_shim::TempDir;

    // A tiny self-contained temp-dir helper so this module doesn't need a
    // dev-dependency just for a handful of file round-trip tests.
    mod tempfile_shim {
        use std::path::{Path, PathBuf};

        pub struct TempDir(PathBuf);

        impl TempDir {
            pub fn new(label: &str) -> Self {
                let dir = std::env::temp_dir().join(format!(
                    "mvs-version-sources-{label}-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or_default()
                ));
                std::fs::create_dir_all(&dir).expect("create temp dir");
                Self(dir)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn identity(arch: u64, feat: u64, prot: u64, fix: u64) -> Identity {
        Identity {
            mvs: Identity::format_mvs(arch, feat, prot, fix, "cli"),
            arch,
            feat,
            prot,
            fix,
            cont: "cli".to_string(),
        }
    }

    #[test]
    fn cargo_toml_reads_and_writes_package_section_only() {
        let content =
            "[package]\nname = \"demo\"\nversion = \"1.2.3\"\n\n[dependencies]\nserde = \"1.0\"\n";
        let span = locate_scoped_toml_version(content, "package").expect("found");
        assert_eq!(&content[span], "1.2.3");

        let dir = TempDir::new("cargo");
        let path = dir.path().join("Cargo.toml");
        fs::write(&path, content).unwrap();
        let entry = make_entry("Cargo.toml", VersionFileKind::CargoToml);

        assert_eq!(read_version(dir.path(), &entry).unwrap(), "1.2.3");
        assert!(write_version(dir.path(), &entry, "2.0.0").unwrap());
        assert_eq!(read_version(dir.path(), &entry).unwrap(), "2.0.0");
        // Idempotent: a second write with the same value reports no change.
        assert!(!write_version(dir.path(), &entry, "2.0.0").unwrap());
        let rewritten = fs::read_to_string(&path).unwrap();
        assert!(rewritten.contains("serde = \"1.0\""));
        assert!(!rewritten.contains("1.2.3"));
    }

    #[test]
    fn cargo_toml_version_with_trailing_comment_is_detected() {
        // Regression guard: found via a real-world dry run against
        // rust-lang/log, whose Cargo.toml has
        // `version = "0.4.34" # remember to update html_root_url`. Rust's
        // `regex` crate doesn't special-case `$` before a trailing `\n` the
        // way e.g. Python's `re` does, so a naive per-line `$`-anchored
        // match silently failed on any version line followed by a comment.
        let content = "[package]\nname = \"log\"\nversion = \"0.4.34\" # remember to update html_root_url\nauthors = []\n";
        let span = locate_scoped_toml_version(content, "package").expect("found");
        assert_eq!(&content[span], "0.4.34");

        let dir = TempDir::new("cargo-trailing-comment");
        let path = dir.path().join("Cargo.toml");
        fs::write(&path, content).unwrap();
        let entry = make_entry("Cargo.toml", VersionFileKind::CargoToml);
        assert_eq!(read_version(dir.path(), &entry).unwrap(), "0.4.34");
        assert!(write_version(dir.path(), &entry, "0.5.0").unwrap());
        let rewritten = fs::read_to_string(&path).unwrap();
        assert_eq!(
            rewritten,
            "[package]\nname = \"log\"\nversion = \"0.5.0\" # remember to update html_root_url\nauthors = []\n"
        );
    }

    #[test]
    fn cargo_toml_ignores_dependency_version_before_package_section() {
        // Regression guard: a `[dependencies]` table appearing anywhere must
        // never be mistaken for `[package]`.
        let content =
            "[dependencies]\nserde = \"1.0\"\n\n[package]\nname = \"demo\"\nversion = \"0.1.0\"\n";
        let span = locate_scoped_toml_version(content, "package").expect("found");
        assert_eq!(&content[span], "0.1.0");
    }

    #[test]
    fn cargo_toml_falls_back_to_workspace_package_section() {
        let content =
            "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.package]\nversion = \"3.4.5\"\n";
        let span = locate_version_span(VersionFileKind::CargoToml, content).expect("found");
        assert_eq!(&content[span], "3.4.5");
    }

    #[test]
    fn cargo_toml_workspace_inherited_version_is_not_editable() {
        let content = "[package]\nname = \"demo\"\nversion.workspace = true\n";
        assert!(locate_scoped_toml_version(content, "package").is_none());
    }

    #[test]
    fn npm_package_json_ignores_nested_version_keys() {
        let content = r#"{
  "name": "demo",
  "config": {
    "version": "should-not-match"
  },
  "version": "1.4.0",
  "engines": { "node": ">=18" }
}
"#;
        let span = locate_json_top_level_string_field(content, "version").expect("found");
        assert_eq!(&content[span], "1.4.0");

        let dir = TempDir::new("npm");
        fs::write(dir.path().join("package.json"), content).unwrap();
        let entry = make_entry("package.json", VersionFileKind::NpmPackageJson);
        assert_eq!(read_version(dir.path(), &entry).unwrap(), "1.4.0");
        assert!(write_version(dir.path(), &entry, "1.5.0").unwrap());
        let rewritten = fs::read_to_string(dir.path().join("package.json")).unwrap();
        assert!(rewritten.contains("\"version\": \"1.5.0\""));
        assert!(rewritten.contains("\"version\": \"should-not-match\""));
    }

    #[test]
    fn pyproject_sniffs_poetry_vs_pep621() {
        let poetry = "[tool.poetry]\nname = \"demo\"\nversion = \"0.3.0\"\n";
        assert!(locate_scoped_toml_version(poetry, "tool.poetry").is_some());
        assert!(locate_scoped_toml_version(poetry, "project").is_none());

        let pep621 = "[project]\nname = \"demo\"\nversion = \"0.3.0\"\n";
        assert!(locate_scoped_toml_version(pep621, "project").is_some());
    }

    #[test]
    fn pubspec_yaml_preserves_build_suffix() {
        let content = "name: demo\nversion: 1.2.3+7\nenvironment:\n  sdk: '>=3.0.0'\n";
        let dir = TempDir::new("pubspec");
        fs::write(dir.path().join("pubspec.yaml"), content).unwrap();
        let entry = make_entry("pubspec.yaml", VersionFileKind::PubspecYaml);

        assert_eq!(read_version(dir.path(), &entry).unwrap(), "1.2.3");
        assert!(write_version(dir.path(), &entry, "1.3.0").unwrap());
        let rewritten = fs::read_to_string(dir.path().join("pubspec.yaml")).unwrap();
        assert!(rewritten.contains("version: 1.3.0+7"));
    }

    #[test]
    fn gradle_properties_reads_top_level_key_only() {
        let content = "org.gradle.jvmargs=-Xmx2g\nversion=2.1.0\n";
        let span = locate_top_level_line_value(content, "version=").expect("found");
        assert_eq!(&content[span], "2.1.0");
    }

    #[test]
    fn build_gradle_kts_matches_quoted_assignment() {
        let content =
            "plugins {\n    kotlin(\"jvm\")\n}\n\nversion = \"1.0.1\"\ngroup = \"com.example\"\n";
        let span = locate_version_span(VersionFileKind::BuildGradle, content).expect("found");
        assert_eq!(&content[span], "1.0.1");
    }

    #[test]
    fn pom_xml_ignores_parent_and_dependency_versions() {
        let content = r#"<project>
  <parent>
    <groupId>org.example</groupId>
    <artifactId>parent</artifactId>
    <version>9.9.9</version>
  </parent>
  <groupId>org.example</groupId>
  <artifactId>demo</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>org.other</groupId>
      <artifactId>lib</artifactId>
      <version>4.5.6</version>
    </dependency>
  </dependencies>
</project>
"#;
        let span = locate_pom_version(content).expect("found");
        assert_eq!(&content[span], "1.0.0");

        let dir = TempDir::new("pom");
        fs::write(dir.path().join("pom.xml"), content).unwrap();
        let entry = make_entry("pom.xml", VersionFileKind::PomXml);
        assert!(write_version(dir.path(), &entry, "1.1.0").unwrap());
        let rewritten = fs::read_to_string(dir.path().join("pom.xml")).unwrap();
        assert!(rewritten.contains("<version>1.1.0</version>"));
        assert!(rewritten.contains("<version>9.9.9</version>"));
        assert!(rewritten.contains("<version>4.5.6</version>"));
    }

    #[test]
    fn csproj_reads_version_element() {
        let content = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <Version>2.3.4</Version>\n  </PropertyGroup>\n</Project>\n";
        let span = locate_version_span(VersionFileKind::Csproj, content).expect("found");
        assert_eq!(&content[span], "2.3.4");
    }

    #[test]
    fn gemspec_and_ruby_constant_and_rockspec() {
        let gemspec = "Gem::Specification.new do |spec|\n  spec.version = \"0.5.0\"\nend\n";
        let span = locate_version_span(VersionFileKind::GemspecLiteral, gemspec).expect("found");
        assert_eq!(&gemspec[span], "0.5.0");

        let ruby_const = "module Demo\n  VERSION = \"0.5.0\"\nend\n";
        let span =
            locate_version_span(VersionFileKind::RubyVersionConstant, ruby_const).expect("found");
        assert_eq!(&ruby_const[span], "0.5.0");

        let rockspec = "package = \"demo\"\nversion = \"1.2.0-1\"\n";
        let span = locate_version_span(VersionFileKind::LuaRockspec, rockspec).expect("found");
        assert_eq!(&rockspec[span], "1.2.0");
    }

    #[test]
    fn plain_version_file_round_trips_trailing_newline() {
        let dir = TempDir::new("plain");
        fs::write(dir.path().join("VERSION"), "1.0.0\n").unwrap();
        let entry = make_entry("VERSION", VersionFileKind::PlainVersionFile);
        assert_eq!(read_version(dir.path(), &entry).unwrap(), "1.0.0");
        assert!(write_version(dir.path(), &entry, "1.1.0").unwrap());
        assert_eq!(
            fs::read_to_string(dir.path().join("VERSION")).unwrap(),
            "1.1.0\n"
        );
    }

    #[test]
    fn detect_finds_cargo_and_skips_files_without_a_locatable_version() {
        let dir = TempDir::new("detect");
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        // A workspace-only Cargo.toml alongside would collide on the same
        // filename in a real project, so this just checks that an unrelated
        // TOML-shaped file with no [package]/[workspace.package] version is
        // not reported.
        fs::write(dir.path().join("composer.json"), "{ \"name\": \"demo\" }\n").unwrap();

        let entries = detect(dir.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "Cargo.toml");
        assert_eq!(entries[0].kind, VersionFileKind::CargoToml);
    }

    #[test]
    fn projected_version_uses_entry_projection() {
        let ident = identity(2, 1, 0, 3);
        let mut entry = make_entry("Cargo.toml", VersionFileKind::CargoToml);
        assert_eq!(projected_version(&entry, &ident), "2.1.3");
        entry.projection = VersionProjection::Full;
        assert_eq!(projected_version(&entry, &ident), "2.1.0.3-cli");
    }
}
