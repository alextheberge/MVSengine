use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use quote::ToTokens;
use regex::Regex;
use syn::{ImplItem, Item, TraitItem, Visibility};
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct TagOccurrence {
    pub name: String,
    pub file: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct ApiSignature {
    pub file: String,
    pub signature: String,
}

#[derive(Debug, Clone, Default)]
pub struct CrawlReport {
    pub feature_tags: BTreeSet<String>,
    pub protocol_tags: BTreeSet<String>,
    pub feature_occurrences: Vec<TagOccurrence>,
    pub protocol_occurrences: Vec<TagOccurrence>,
    pub public_api: Vec<ApiSignature>,
}

struct ApiRegexPack {
    ts_export_decl: Regex,
    ts_export_const: Regex,
    go_func: Regex,
    py_def: Regex,
    java_type: Regex,
    java_method: Regex,
    kt_decl: Regex,
    cs_type: Regex,
    cs_method: Regex,
}

impl ApiRegexPack {
    fn new() -> Result<Self> {
        Ok(Self {
            ts_export_decl: Regex::new(
                r"^\s*export\s+(?:async\s+)?(?P<kind>function|class|interface|type|enum)\s+(?P<rest>.+)$",
            )
            .context("failed to compile TS/JS API regex (decl)")?,
            ts_export_const: Regex::new(
                r"^\s*export\s+const\s+(?P<rest>[A-Za-z0-9_]+(?:\s*:\s*[^=]+)?)\s*=",
            )
            .context("failed to compile TS/JS API regex (const)")?,
            go_func: Regex::new(
                r"^\s*func\s*(?P<recv>\([^)]*\)\s*)?(?P<name>[A-Z][A-Za-z0-9_]*)\s*(?P<sig>\([^)]*\)\s*[^\{]*)",
            )
            .context("failed to compile Go API regex")?,
            py_def: Regex::new(
                r"^\s*def\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?P<sig>\([^)]*\)\s*(?:->\s*[^:]+)?)\s*:",
            )
            .context("failed to compile Python API regex")?,
            java_type: Regex::new(
                r"^\s*public\s+(?:abstract\s+)?(?:final\s+)?(?:class|interface|enum)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
            )
            .context("failed to compile Java API regex (type)")?,
            java_method: Regex::new(
                r"^\s*public\s+(?:static\s+)?(?:final\s+)?[A-Za-z0-9_<>,\[\].?\s]+\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?P<sig>\([^)]*\))",
            )
            .context("failed to compile Java API regex (method)")?,
            kt_decl: Regex::new(
                r"^\s*(?:public\s+)?(?P<kind>class|interface|fun)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?P<rest>[^\{=]*)",
            )
            .context("failed to compile Kotlin API regex")?,
            cs_type: Regex::new(
                r"^\s*public\s+(?:sealed\s+)?(?:static\s+)?(?:class|interface|enum|struct)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
            )
            .context("failed to compile C# API regex (type)")?,
            cs_method: Regex::new(
                r"^\s*public\s+(?:static\s+)?[A-Za-z0-9_<>,\[\].?\s]+\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?P<sig>\([^)]*\))",
            )
            .context("failed to compile C# API regex (method)")?,
        })
    }
}

pub fn crawl_codebase(root: &Path) -> Result<CrawlReport> {
    let feature_re = Regex::new(
        r#"@mvs-feature\s*(?:\(\s*["'](?P<name>[^"']+)["'](?:\s*,\s*(?P<meta>[^)]*))?\)|:\s*(?P<name2>[A-Za-z0-9._-]+))"#,
    )
    .context("failed to compile feature regex")?;
    let protocol_re = Regex::new(
        r#"@mvs-protocol\s*(?:\(\s*["'](?P<surface>[^"']+)["'](?:\s*,\s*(?P<meta>[^)]*))?\)|:\s*(?P<surface2>[A-Za-z0-9._-]+))"#,
    )
    .context("failed to compile protocol regex")?;
    let api_pack = ApiRegexPack::new()?;

    let mut report = CrawlReport::default();

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| !is_excluded(entry))
    {
        let entry = entry.with_context(|| format!("failed walking {}", root.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        if !is_supported_source(path) {
            continue;
        }

        let source = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(_) => continue,
        };

        let rel = relative_display_path(root, path);

        for tag in extract_named_tags(&source, &feature_re, "name", "name2") {
            report.feature_tags.insert(tag.clone());
            report.feature_occurrences.push(TagOccurrence {
                name: tag,
                file: rel.clone(),
            });
        }

        for tag in extract_named_tags(&source, &protocol_re, "surface", "surface2") {
            report.protocol_tags.insert(tag.clone());
            report.protocol_occurrences.push(TagOccurrence {
                name: tag,
                file: rel.clone(),
            });
        }

        let signatures = extract_public_api(path, &source, &api_pack);
        for signature in signatures {
            report.public_api.push(ApiSignature {
                file: rel.clone(),
                signature,
            });
        }
    }

    report.feature_occurrences.sort();
    report.protocol_occurrences.sort();
    report.public_api.sort();
    report.public_api.dedup();

    Ok(report)
}

fn extract_named_tags(
    source: &str,
    regex: &Regex,
    primary_group: &str,
    fallback_group: &str,
) -> Vec<String> {
    let mut values = Vec::new();

    for capture in regex.captures_iter(source) {
        let tag = capture
            .name(primary_group)
            .or_else(|| capture.name(fallback_group))
            .map(|value| value.as_str().trim().to_string())
            .unwrap_or_default();

        if !tag.is_empty() {
            values.push(tag);
        }
    }

    values
}

fn extract_public_api(path: &Path, source: &str, regex: &ApiRegexPack) -> Vec<String> {
    if matches!(path.extension().and_then(|ext| ext.to_str()), Some("rs")) {
        if let Some(ast_signatures) = extract_rust_public_api(source) {
            if !ast_signatures.is_empty() {
                return ast_signatures;
            }
        }
    }

    extract_regex_public_api(path, source, regex)
}

fn extract_regex_public_api(path: &Path, source: &str, regex: &ApiRegexPack) -> Vec<String> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let mut signatures = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || is_comment_line(trimmed, extension) {
            continue;
        }

        match extension {
            "ts" | "tsx" | "js" | "jsx" => {
                if let Some(capture) = regex.ts_export_decl.captures(trimmed) {
                    let kind = capture
                        .name("kind")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let rest = capture
                        .name("rest")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let rest = rest.split('{').next().unwrap_or(rest).trim();
                    let signature = normalize_signature(&format!("{kind} {rest}"));
                    if !signature.is_empty() {
                        signatures.push(format!("ts/js:{signature}"));
                    }
                }

                if let Some(capture) = regex.ts_export_const.captures(trimmed) {
                    let rest = capture
                        .name("rest")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let signature = normalize_signature(&format!("const {rest}"));
                    if !signature.is_empty() {
                        signatures.push(format!("ts/js:{signature}"));
                    }
                }
            }
            "go" => {
                if let Some(capture) = regex.go_func.captures(trimmed) {
                    let recv = capture
                        .name("recv")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let name = capture
                        .name("name")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let sig = capture
                        .name("sig")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let signature = normalize_signature(&format!("func {recv}{name}{sig}"));
                    if !signature.is_empty() {
                        signatures.push(format!("go:{signature}"));
                    }
                }
            }
            "py" => {
                if let Some(capture) = regex.py_def.captures(trimmed) {
                    let name = capture
                        .name("name")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    if name.starts_with('_') {
                        continue;
                    }
                    let sig = capture
                        .name("sig")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let signature = normalize_signature(&format!("def {name}{sig}"));
                    if !signature.is_empty() {
                        signatures.push(format!("python:{signature}"));
                    }
                }
            }
            "java" => {
                if let Some(capture) = regex.java_type.captures(trimmed) {
                    if let Some(name) = capture.name("name") {
                        signatures.push(format!("java:type {}", name.as_str()));
                    }
                }
                if let Some(capture) = regex.java_method.captures(trimmed) {
                    let name = capture
                        .name("name")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let sig = capture
                        .name("sig")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    signatures.push(format!("java:method {}{}", name, normalize_signature(sig)));
                }
            }
            "kt" => {
                if let Some(capture) = regex.kt_decl.captures(trimmed) {
                    let kind = capture
                        .name("kind")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let name = capture
                        .name("name")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let rest = capture
                        .name("rest")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let signature = normalize_signature(&format!("{kind} {name} {rest}"));
                    if !signature.is_empty() {
                        signatures.push(format!("kotlin:{signature}"));
                    }
                }
            }
            "cs" => {
                if let Some(capture) = regex.cs_type.captures(trimmed) {
                    if let Some(name) = capture.name("name") {
                        signatures.push(format!("csharp:type {}", name.as_str()));
                    }
                }
                if let Some(capture) = regex.cs_method.captures(trimmed) {
                    let name = capture
                        .name("name")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let sig = capture
                        .name("sig")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    signatures.push(format!(
                        "csharp:method {}{}",
                        name,
                        normalize_signature(sig)
                    ));
                }
            }
            _ => {}
        }
    }

    signatures.sort();
    signatures.dedup();
    signatures
}

fn extract_rust_public_api(source: &str) -> Option<Vec<String>> {
    let parsed = syn::parse_file(source).ok()?;
    let mut signatures = Vec::new();
    collect_rust_items(&parsed.items, "", &mut signatures);
    signatures.sort();
    signatures.dedup();
    Some(signatures)
}

fn collect_rust_items(items: &[Item], module_prefix: &str, signatures: &mut Vec<String>) {
    for item in items {
        match item {
            Item::Fn(item_fn) if is_public(&item_fn.vis) => {
                let sig = normalize_signature(&item_fn.sig.to_token_stream().to_string());
                signatures.push(format!("rust:fn {module_prefix}{sig}"));
            }
            Item::Struct(item_struct) if is_public(&item_struct.vis) => {
                let sig = format!(
                    "{}{}",
                    item_struct.ident,
                    normalize_signature(&item_struct.generics.to_token_stream().to_string())
                );
                signatures.push(format!("rust:struct {module_prefix}{sig}"));
            }
            Item::Enum(item_enum) if is_public(&item_enum.vis) => {
                let sig = format!(
                    "{}{}",
                    item_enum.ident,
                    normalize_signature(&item_enum.generics.to_token_stream().to_string())
                );
                signatures.push(format!("rust:enum {module_prefix}{sig}"));
            }
            Item::Trait(item_trait) if is_public(&item_trait.vis) => {
                let trait_name = format!("{}{name}", module_prefix, name = item_trait.ident);
                signatures.push(format!("rust:trait {trait_name}"));
                for trait_item in &item_trait.items {
                    if let TraitItem::Fn(method) = trait_item {
                        let sig = normalize_signature(&method.sig.to_token_stream().to_string());
                        signatures.push(format!("rust:trait-fn {trait_name}::{sig}"));
                    }
                }
            }
            Item::Type(item_type) if is_public(&item_type.vis) => {
                let ty = normalize_signature(&item_type.ty.to_token_stream().to_string());
                signatures.push(format!("rust:type {module_prefix}{}={ty}", item_type.ident));
            }
            Item::Const(item_const) if is_public(&item_const.vis) => {
                let ty = normalize_signature(&item_const.ty.to_token_stream().to_string());
                signatures.push(format!(
                    "rust:const {module_prefix}{}:{ty}",
                    item_const.ident
                ));
            }
            Item::Static(item_static) if is_public(&item_static.vis) => {
                let ty = normalize_signature(&item_static.ty.to_token_stream().to_string());
                signatures.push(format!(
                    "rust:static {module_prefix}{}:{ty}",
                    item_static.ident
                ));
            }
            Item::Impl(item_impl) => {
                let impl_scope = item_impl
                    .trait_
                    .as_ref()
                    .map(|(_, path, _)| normalize_signature(&path.to_token_stream().to_string()))
                    .unwrap_or_else(|| {
                        normalize_signature(&item_impl.self_ty.to_token_stream().to_string())
                    });

                for impl_item in &item_impl.items {
                    if let ImplItem::Fn(method) = impl_item {
                        if !is_public(&method.vis) {
                            continue;
                        }

                        let sig = normalize_signature(&method.sig.to_token_stream().to_string());
                        signatures.push(format!("rust:impl-fn {module_prefix}{impl_scope}::{sig}"));
                    }
                }
            }
            Item::Mod(item_mod) if is_public(&item_mod.vis) => {
                signatures.push(format!("rust:mod {module_prefix}{}", item_mod.ident));
                if let Some((_, nested_items)) = &item_mod.content {
                    let nested_prefix = format!("{}{name}::", module_prefix, name = item_mod.ident);
                    collect_rust_items(nested_items, &nested_prefix, signatures);
                }
            }
            _ => {}
        }
    }
}

fn is_public(visibility: &Visibility) -> bool {
    matches!(visibility, Visibility::Public(_))
}

fn normalize_signature(raw: &str) -> String {
    let mut normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    for (from, to) in [
        (" (", "("),
        ("( ", "("),
        (" )", ")"),
        (" ,", ","),
        (" ;", ";"),
        (" :", ":"),
        (" ::", "::"),
        (" < ", "<"),
        (" >", ">"),
        (" = ", "="),
        (" & ", "&"),
    ] {
        normalized = normalized.replace(from, to);
    }
    normalized
}

fn is_comment_line(line: &str, extension: &str) -> bool {
    match extension {
        "py" => line.starts_with('#'),
        _ => line.starts_with("//") || line.starts_with("/*") || line.starts_with('*'),
    }
}

fn is_excluded(entry: &DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }

    matches!(
        entry.file_name().to_str(),
        Some(
            ".git"
                | "node_modules"
                | "dist"
                | "build"
                | "target"
                | "vendor"
                | ".next"
                | "tests"
                | "examples"
                | "benches"
        )
    )
}

fn is_supported_source(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("rs" | "ts" | "tsx" | "js" | "jsx" | "go" | "py" | "java" | "kt" | "cs")
    )
}

fn relative_display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

#[allow(dead_code)]
fn normalize_path(path: &Path) -> PathBuf {
    path.components().collect()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::crawl_codebase;

    struct TempWorkspace {
        path: PathBuf,
    }

    impl TempWorkspace {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let index = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mvs-crawler-test-{}-{}-{}",
                std::process::id(),
                nanos,
                index
            ));
            fs::create_dir_all(&path).expect("failed to create temp workspace");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn crawls_tags_and_public_api() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.ts"),
            r#"
            // @mvs-feature("offline_storage")
            // @mvs-protocol("auth-api-v1")
            export function login(username: string): Promise<string> {
              return Promise.resolve(username)
            }
            export interface Session {
              token: string
            }
        "#,
        )
        .expect("failed to write ts fixture");

        let report = crawl_codebase(workspace.path()).expect("crawler failed");

        assert!(report.feature_tags.contains("offline_storage"));
        assert!(report.protocol_tags.contains("auth-api-v1"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("login")));
    }

    #[test]
    fn rust_ast_extraction_tracks_pub_signatures() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("lib.rs"),
            r#"
            /// @mvs-feature("core_runtime")
            /// @mvs-protocol("rust_host_surface")
            pub fn handshake(version: u32) -> bool { version > 0 }

            pub struct HostAdapter;

            impl HostAdapter {
                pub fn connect(&self, target: &str) -> bool { !target.is_empty() }
                fn private_method(&self) {}
            }
        "#,
        )
        .expect("failed to write rust fixture");

        let report = crawl_codebase(workspace.path()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("fn handshake")));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("HostAdapter::fn connect")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("private_method")));
    }

    #[test]
    fn excludes_tests_directory_by_default() {
        let workspace = TempWorkspace::new();
        let tests_dir = workspace.path().join("tests");
        fs::create_dir_all(&tests_dir).expect("failed to create tests dir");
        fs::write(
            tests_dir.join("api.ts"),
            "// @mvs-feature(\"test_only\")\nexport function helper() {}\n",
        )
        .expect("failed to write tests file");

        let report = crawl_codebase(workspace.path()).expect("crawler failed");

        assert!(report.feature_tags.is_empty());
        assert!(report.public_api.is_empty());
    }
}
