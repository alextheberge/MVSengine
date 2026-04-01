// SPDX-License-Identifier: AGPL-3.0-only
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use quote::ToTokens;
use regex::Regex;
use syn::{FnArg, ImplItem, Item, ReturnType, Signature, TraitItem, Visibility};
use tree_sitter::{Language as TreeSitterLanguage, Node, Parser as TreeSitterParser};
use walkdir::{DirEntry, WalkDir};

use crate::mvs::manifest::ScanPolicy;

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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum SourceLanguage {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Jsx,
    Go,
    Python,
    Java,
    Kotlin,
    Csharp,
    Php,
    Swift,
    Luau,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum LexStrategy {
    CStyle,
    Python,
    Php,
    Luau,
}

#[derive(Debug, Clone)]
struct LexedSource {
    comments: Vec<String>,
    masked_code: String,
}

struct ApiRegexPack {
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

impl SourceLanguage {
    fn from_path(path: &Path) -> Option<Self> {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("rs") => Some(Self::Rust),
            Some("ts") => Some(Self::TypeScript),
            Some("tsx") => Some(Self::Tsx),
            Some("js") => Some(Self::JavaScript),
            Some("jsx") => Some(Self::Jsx),
            Some("go") => Some(Self::Go),
            Some("py") => Some(Self::Python),
            Some("java") => Some(Self::Java),
            Some("kt") => Some(Self::Kotlin),
            Some("cs") => Some(Self::Csharp),
            Some("php") => Some(Self::Php),
            Some("swift") => Some(Self::Swift),
            Some("luau") => Some(Self::Luau),
            _ => None,
        }
    }

    fn extension_label(self) -> &'static str {
        match self {
            Self::Rust => "rs",
            Self::TypeScript => "ts",
            Self::Tsx => "tsx",
            Self::JavaScript => "js",
            Self::Jsx => "jsx",
            Self::Go => "go",
            Self::Python => "py",
            Self::Java => "java",
            Self::Kotlin => "kt",
            Self::Csharp => "cs",
            Self::Php => "php",
            Self::Swift => "swift",
            Self::Luau => "luau",
        }
    }

    fn lex_strategy(self) -> LexStrategy {
        match self {
            Self::Python => LexStrategy::Python,
            Self::Php => LexStrategy::Php,
            Self::Luau => LexStrategy::Luau,
            _ => LexStrategy::CStyle,
        }
    }

    fn uses_nested_block_comments(self) -> bool {
        matches!(self, Self::Rust | Self::Swift)
    }

    fn tree_sitter_language(self) -> Option<TreeSitterLanguage> {
        match self {
            Self::Go => Some(tree_sitter_go::LANGUAGE.into()),
            Self::Python => Some(tree_sitter_python::LANGUAGE.into()),
            Self::Java => Some(tree_sitter_java::LANGUAGE.into()),
            Self::Kotlin => Some(tree_sitter_kotlin_ng::LANGUAGE.into()),
            Self::Csharp => Some(tree_sitter_c_sharp::LANGUAGE.into()),
            Self::Php => Some(tree_sitter_php::LANGUAGE_PHP.into()),
            Self::Swift => Some(tree_sitter_swift::LANGUAGE.into()),
            Self::Luau => Some(tree_sitter_luau::LANGUAGE.into()),
            Self::TypeScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
            Self::Tsx => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
            Self::JavaScript | Self::Jsx => Some(tree_sitter_javascript::LANGUAGE.into()),
            Self::Rust => None,
        }
    }

    fn extract_tree_sitter_public_api(self, root: Node<'_>, source: &str) -> Vec<String> {
        match self {
            Self::TypeScript | Self::Tsx | Self::JavaScript | Self::Jsx => {
                extract_tree_sitter_ts_js_public_api(root, source)
            }
            Self::Go => extract_tree_sitter_go_public_api(root, source),
            Self::Python => extract_tree_sitter_python_public_api(root, source),
            Self::Java => extract_tree_sitter_java_public_api(root, source),
            Self::Kotlin => extract_tree_sitter_kotlin_public_api(root, source),
            Self::Csharp => extract_tree_sitter_csharp_public_api(root, source),
            Self::Php => extract_tree_sitter_php_public_api(root, source),
            Self::Swift => extract_tree_sitter_swift_public_api(root, source),
            Self::Luau => extract_tree_sitter_luau_public_api(root, source),
            Self::Rust => Vec::new(),
        }
    }
}

pub fn crawl_codebase(root: &Path, scan_policy: &ScanPolicy) -> Result<CrawlReport> {
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
        let Some(language) = SourceLanguage::from_path(path) else {
            continue;
        };
        let rel = relative_display_path(root, path);
        if scan_policy.is_excluded(&rel) {
            continue;
        }

        let source = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let lexed = lex_source(&source, language);

        for tag in extract_named_tags(&lexed.comments, &feature_re, "name", "name2") {
            report.feature_tags.insert(tag.clone());
            report.feature_occurrences.push(TagOccurrence {
                name: tag,
                file: rel.clone(),
            });
        }

        for tag in extract_named_tags(&lexed.comments, &protocol_re, "surface", "surface2") {
            report.protocol_tags.insert(tag.clone());
            report.protocol_occurrences.push(TagOccurrence {
                name: tag,
                file: rel.clone(),
            });
        }

        if scan_policy.includes_public_api(&rel) {
            let signatures = extract_public_api(language, &source, &lexed.masked_code, &api_pack);
            for signature in signatures {
                if !scan_policy.includes_public_api_item(&rel, &signature) {
                    continue;
                }
                report.public_api.push(ApiSignature {
                    file: rel.clone(),
                    signature,
                });
            }
        }
    }

    report.feature_occurrences.sort();
    report.protocol_occurrences.sort();
    report.public_api.sort();
    report.public_api.dedup();

    Ok(report)
}

fn extract_named_tags(
    comments: &[String],
    regex: &Regex,
    primary_group: &str,
    fallback_group: &str,
) -> Vec<String> {
    let mut values = Vec::new();

    for comment in comments {
        for capture in regex.captures_iter(comment) {
            let tag = capture
                .name(primary_group)
                .or_else(|| capture.name(fallback_group))
                .map(|value| value.as_str().trim().to_string())
                .unwrap_or_default();

            if !tag.is_empty() {
                values.push(tag);
            }
        }
    }

    values
}

fn extract_public_api(
    language: SourceLanguage,
    source: &str,
    masked_code: &str,
    regex: &ApiRegexPack,
) -> Vec<String> {
    if language == SourceLanguage::Rust {
        if let Some(ast_signatures) = extract_rust_public_api(source) {
            if !ast_signatures.is_empty() {
                return ast_signatures;
            }
        }
    }

    if let Some(tree_sitter_signatures) = extract_tree_sitter_public_api(language, source) {
        return tree_sitter_signatures;
    }

    extract_regex_public_api(language, masked_code, regex)
}

fn extract_tree_sitter_public_api(language: SourceLanguage, source: &str) -> Option<Vec<String>> {
    let grammar = language.tree_sitter_language()?;
    let mut parser = TreeSitterParser::new();
    parser.set_language(&grammar).ok()?;
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();

    let mut signatures = language.extract_tree_sitter_public_api(root, source);
    signatures.sort();
    signatures.dedup();
    Some(signatures)
}

fn extract_tree_sitter_ts_js_public_api(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    for child in named_children(root) {
        if child.kind() != "export_statement" {
            continue;
        }

        signatures.extend(extract_tree_sitter_export_statement(child, source));
    }

    signatures
}

fn extract_tree_sitter_go_public_api(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();

    for child in named_children(root) {
        match child.kind() {
            "function_declaration" | "method_declaration"
                if is_exported_tree_sitter_name(child, source) =>
            {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                {
                    signatures.push(format!("go:{signature}"));
                }
            }
            _ => {}
        }
    }

    signatures
}

fn extract_tree_sitter_python_public_api(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_tree_sitter_python_public_api(root, source, &mut signatures, false);
    signatures
}

fn collect_tree_sitter_python_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    inside_callable: bool,
) {
    for child in named_children(node) {
        collect_tree_sitter_python_definition(child, source, signatures, inside_callable);
    }
}

fn collect_tree_sitter_python_definition(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    inside_callable: bool,
) {
    match node.kind() {
        "function_definition" => {
            if !inside_callable && is_public_python_name(node, source) {
                if let Some(signature) = extract_tree_sitter_prefix_signature(node, source, "body")
                {
                    signatures.push(format!("python:{signature}"));
                }
            }
        }
        "class_definition" => {
            if !inside_callable {
                if let Some(body) = node.child_by_field_name("body") {
                    collect_tree_sitter_python_public_api(body, source, signatures, false);
                }
            }
        }
        "decorated_definition" => {
            if let Some(definition) = node.child_by_field_name("definition") {
                collect_tree_sitter_python_definition(
                    definition,
                    source,
                    signatures,
                    inside_callable,
                );
            }
        }
        "module" | "block" => {
            collect_tree_sitter_python_public_api(node, source, signatures, inside_callable);
        }
        _ => {}
    }
}

fn extract_tree_sitter_java_public_api(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_tree_sitter_java_public_api(root, source, &mut signatures);
    signatures
}

fn collect_tree_sitter_java_public_api(node: Node<'_>, source: &str, signatures: &mut Vec<String>) {
    for child in named_children(node) {
        match child.kind() {
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "annotation_type_declaration" => {
                if has_tree_sitter_keyword(child, source, "body", "public") {
                    if let Some(signature) =
                        extract_tree_sitter_prefix_signature(child, source, "body")
                            .and_then(|value| trim_signature_to_keywords(&value, &["public"]))
                    {
                        signatures.push(format!("java:type {signature}"));
                    }
                }

                if let Some(body) = child.child_by_field_name("body") {
                    collect_tree_sitter_java_public_api(body, source, signatures);
                }
            }
            "method_declaration" => {
                if has_tree_sitter_keyword(child, source, "body", "public") {
                    if let Some(signature) =
                        extract_tree_sitter_prefix_signature(child, source, "body")
                            .and_then(|value| trim_signature_to_keywords(&value, &["public"]))
                    {
                        signatures.push(format!("java:method {signature}"));
                    }
                }
            }
            "class_body" | "interface_body" | "annotation_type_body" | "enum_body_declarations" => {
                collect_tree_sitter_java_public_api(child, source, signatures);
            }
            _ => {}
        }
    }
}

fn extract_tree_sitter_kotlin_public_api(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_tree_sitter_kotlin_public_api(root, source, &mut signatures);
    signatures
}

fn collect_tree_sitter_kotlin_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
) {
    for child in named_children(node) {
        match child.kind() {
            "class_declaration" | "object_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_before_named_child(
                    child,
                    source,
                    &["class_body", "enum_class_body"],
                )
                .and_then(|value| normalize_kotlin_signature(&value))
                {
                    signatures.push(format!("kotlin:{signature}"));
                }

                for nested in named_children(child) {
                    if matches!(nested.kind(), "class_body" | "enum_class_body") {
                        collect_tree_sitter_kotlin_public_api(nested, source, signatures);
                    }
                }
            }
            "function_declaration" => {
                if let Some(signature) =
                    extract_tree_sitter_prefix_before_named_child(child, source, &["function_body"])
                        .and_then(|value| normalize_kotlin_signature(&value))
                {
                    signatures.push(format!("kotlin:{signature}"));
                }
            }
            "class_body" | "enum_class_body" => {
                collect_tree_sitter_kotlin_public_api(child, source, signatures);
            }
            _ => {}
        }
    }
}

fn extract_tree_sitter_csharp_public_api(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_tree_sitter_csharp_public_api(root, source, &mut signatures);
    signatures
}

fn collect_tree_sitter_csharp_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
) {
    for child in named_children(node) {
        match child.kind() {
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "struct_declaration" => {
                if has_tree_sitter_keyword(child, source, "body", "public") {
                    if let Some(signature) =
                        extract_tree_sitter_prefix_signature(child, source, "body")
                            .and_then(|value| trim_signature_to_keywords(&value, &["public"]))
                    {
                        signatures.push(format!("csharp:type {signature}"));
                    }
                }

                if let Some(body) = child.child_by_field_name("body") {
                    collect_tree_sitter_csharp_public_api(body, source, signatures);
                }
            }
            "method_declaration" => {
                if has_tree_sitter_keyword(child, source, "body", "public") {
                    if let Some(signature) =
                        extract_tree_sitter_prefix_signature(child, source, "body")
                            .and_then(|value| trim_signature_to_keywords(&value, &["public"]))
                    {
                        signatures.push(format!("csharp:method {signature}"));
                    }
                }
            }
            "namespace_declaration" => {
                if let Some(body) = child.child_by_field_name("body") {
                    collect_tree_sitter_csharp_public_api(body, source, signatures);
                }
            }
            "compilation_unit" | "declaration_list" | "file_scoped_namespace_declaration" => {
                collect_tree_sitter_csharp_public_api(child, source, signatures);
            }
            _ => {}
        }
    }
}

fn extract_tree_sitter_php_public_api(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_tree_sitter_php_public_api(root, source, &mut signatures, false);
    signatures
}

fn collect_tree_sitter_php_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    inside_interface: bool,
) {
    for child in named_children(node) {
        match child.kind() {
            "class_declaration" | "trait_declaration" | "enum_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_php_type_signature(&value))
                {
                    signatures.push(format!("php:{signature}"));
                }

                if let Some(body) = child.child_by_field_name("body") {
                    collect_tree_sitter_php_public_api(body, source, signatures, false);
                }
            }
            "interface_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_php_type_signature(&value))
                {
                    signatures.push(format!("php:{signature}"));
                }

                if let Some(body) = child.child_by_field_name("body") {
                    collect_tree_sitter_php_public_api(body, source, signatures, true);
                }
            }
            "function_definition" => {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_php_function_signature(&value))
                {
                    signatures.push(format!("php:{signature}"));
                }
            }
            "method_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_php_method_signature(&value, inside_interface))
                {
                    signatures.push(format!("php:{signature}"));
                }
            }
            "property_declaration" => {
                signatures.extend(extract_tree_sitter_php_property_signatures(
                    child,
                    source,
                    inside_interface,
                ));
            }
            "const_declaration" => {
                signatures.extend(extract_tree_sitter_php_const_signatures(
                    child,
                    source,
                    inside_interface,
                ));
            }
            "program" | "compound_statement" | "declaration_list" => {
                collect_tree_sitter_php_public_api(child, source, signatures, inside_interface);
            }
            "namespace_definition" => {
                if let Some(body) = child.child_by_field_name("body") {
                    collect_tree_sitter_php_public_api(body, source, signatures, inside_interface);
                }
            }
            _ => {}
        }
    }
}

fn extract_tree_sitter_swift_public_api(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_tree_sitter_swift_public_api(root, source, &mut signatures, None);
    signatures
}

fn collect_tree_sitter_swift_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    exported_protocol_visibility: Option<&str>,
) {
    for child in named_children(node) {
        match child.kind() {
            "class_declaration"
            | "struct_declaration"
            | "enum_declaration"
            | "extension_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_swift_signature(&value))
                {
                    signatures.push(format!("swift:{signature}"));
                }

                if let Some(body) = child.child_by_field_name("body") {
                    collect_tree_sitter_swift_public_api(body, source, signatures, None);
                }
            }
            "protocol_declaration" => {
                let normalized = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_swift_signature(&value));
                let visibility = normalized.as_deref().and_then(swift_visibility_keyword);

                if let Some(signature) = normalized.as_deref() {
                    signatures.push(format!("swift:{signature}"));
                }

                if let Some(body) = child.child_by_field_name("body") {
                    collect_tree_sitter_swift_public_api(body, source, signatures, visibility);
                }
            }
            "function_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_swift_signature(&value))
                {
                    signatures.push(format!("swift:{signature}"));
                }
            }
            "property_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_before_fields(
                    child,
                    source,
                    &["computed_value", "value"],
                )
                .and_then(|value| normalize_swift_signature(&value))
                {
                    signatures.push(format!("swift:{signature}"));
                }
            }
            "protocol_function_declaration" => {
                if let Some(signature) =
                    extract_tree_sitter_prefix_before_named_child(child, source, &["statements"])
                        .and_then(|value| {
                            normalize_swift_protocol_member_signature(
                                &value,
                                exported_protocol_visibility,
                            )
                        })
                {
                    signatures.push(format!("swift:{signature}"));
                }
            }
            "protocol_property_declaration" => {
                if let Some(signature) = node_text(child, source)
                    .map(normalize_tree_sitter_signature)
                    .and_then(|value| {
                        normalize_swift_protocol_member_signature(
                            &value,
                            exported_protocol_visibility,
                        )
                    })
                {
                    signatures.push(format!("swift:{signature}"));
                }
            }
            "source_file" | "class_body" | "protocol_body" | "enum_class_body" => {
                collect_tree_sitter_swift_public_api(
                    child,
                    source,
                    signatures,
                    exported_protocol_visibility,
                );
            }
            _ => {}
        }
    }
}

fn extract_tree_sitter_luau_public_api(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_tree_sitter_luau_global_public_api(root, source, &mut signatures);
    collect_tree_sitter_luau_module_exports(root, source, &mut signatures);
    signatures
}

fn collect_tree_sitter_luau_global_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
) {
    for child in named_children(node) {
        match child.kind() {
            "chunk" | "statement" => {
                collect_tree_sitter_luau_global_public_api(child, source, signatures);
            }
            "function_declaration" => {
                let Some(name) = child.child_by_field_name("name") else {
                    continue;
                };
                if name.kind() != "identifier" {
                    continue;
                }

                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .filter(|value| !value.starts_with("local function "))
                {
                    signatures.push(format!("luau:{signature}"));
                }
            }
            "type_definition" => {
                if let Some(signature) = node_text(child, source)
                    .map(normalize_tree_sitter_signature)
                    .filter(|value| value.starts_with("export type "))
                {
                    signatures.push(format!("luau:{signature}"));
                }
            }
            _ => {}
        }
    }
}

fn collect_tree_sitter_luau_module_exports(
    root: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
) {
    let module_names = extract_luau_returned_module_names(root, source);
    if module_names.is_empty() {
        return;
    }

    for item in luau_top_level_items(root) {
        match item.kind() {
            "variable_declaration" => {
                let Some(assignment) = named_children(item)
                    .into_iter()
                    .find(|child| child.kind() == "assignment_statement")
                else {
                    continue;
                };
                collect_luau_module_assignment_exports(
                    assignment,
                    source,
                    &module_names,
                    signatures,
                );
            }
            "assignment_statement" => {
                collect_luau_module_assignment_exports(item, source, &module_names, signatures);
            }
            "function_declaration" => {
                let Some(name) = item.child_by_field_name("name") else {
                    continue;
                };
                if extract_luau_module_member_name(name, source, &module_names).is_none() {
                    continue;
                }

                if let Some(signature) = extract_tree_sitter_prefix_signature(item, source, "body")
                {
                    signatures.push(format!("luau:{signature}"));
                }
            }
            _ => {}
        }
    }
}

fn collect_luau_module_assignment_exports(
    assignment: Node<'_>,
    source: &str,
    module_names: &BTreeSet<String>,
    signatures: &mut Vec<String>,
) {
    for (target, value) in extract_luau_assignment_pairs(assignment) {
        if let Some(module_name) = extract_luau_identifier_name(target, source) {
            if module_names.contains(module_name) && value.kind() == "table_constructor" {
                collect_luau_table_constructor_exports(module_name, value, source, signatures);
            }
            continue;
        }

        let Some(member_name) = extract_luau_module_member_name(target, source, module_names)
        else {
            continue;
        };

        if value.kind() == "function_definition" {
            if let Some(signature) =
                extract_luau_assigned_function_signature(&member_name, value, source)
            {
                signatures.push(format!("luau:{signature}"));
            }
            continue;
        }

        signatures.push(format!("luau:field {member_name}"));
    }
}

fn collect_luau_table_constructor_exports(
    module_name: &str,
    table_constructor: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
) {
    for field in named_children(table_constructor) {
        if field.kind() != "field" {
            continue;
        }

        let Some(name) = field.child_by_field_name("name") else {
            continue;
        };
        if name.kind() != "identifier" {
            continue;
        }
        let Some(field_name) = node_text(name, source).map(str::trim) else {
            continue;
        };
        if field_name.is_empty() {
            continue;
        }

        let target = format!("{module_name}.{field_name}");
        let Some(value) = field.child_by_field_name("value") else {
            continue;
        };

        if value.kind() == "function_definition" {
            if let Some(signature) =
                extract_luau_assigned_function_signature(&target, value, source)
            {
                signatures.push(format!("luau:{signature}"));
            }
            continue;
        }

        signatures.push(format!("luau:field {target}"));
    }
}

fn extract_luau_returned_module_names(root: Node<'_>, source: &str) -> BTreeSet<String> {
    let mut module_names = BTreeSet::new();

    for item in luau_top_level_items(root) {
        if item.kind() != "return_statement" {
            continue;
        }

        let Some(expression_list) = named_children(item)
            .into_iter()
            .find(|child| child.kind() == "expression_list")
        else {
            continue;
        };

        for expression in named_children(expression_list) {
            if let Some(name) = extract_luau_identifier_name(expression, source) {
                module_names.insert(name.to_string());
            }
        }
    }

    module_names
}

fn luau_top_level_items(node: Node<'_>) -> Vec<Node<'_>> {
    let mut items = Vec::new();

    for child in named_children(node) {
        match child.kind() {
            "chunk" | "statement" => items.extend(luau_top_level_items(child)),
            _ => items.push(child),
        }
    }

    items
}

fn extract_luau_assignment_pairs(assignment: Node<'_>) -> Vec<(Node<'_>, Node<'_>)> {
    let Some(variable_list) = named_children(assignment)
        .into_iter()
        .find(|child| child.kind() == "variable_list")
    else {
        return Vec::new();
    };
    let Some(expression_list) = named_children(assignment)
        .into_iter()
        .find(|child| child.kind() == "expression_list")
    else {
        return Vec::new();
    };

    named_children(variable_list)
        .into_iter()
        .zip(named_children(expression_list))
        .collect()
}

fn extract_luau_identifier_name<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    let node = unwrap_luau_variable(node);
    if node.kind() != "identifier" {
        return None;
    }

    node_text(node, source)
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

fn extract_luau_module_member_name(
    node: Node<'_>,
    source: &str,
    module_names: &BTreeSet<String>,
) -> Option<String> {
    let node = unwrap_luau_variable(node);
    let separator = match node.kind() {
        "dot_index_expression" => ".",
        "method_index_expression" => ":",
        _ => return None,
    };

    let table = node.child_by_field_name("table")?;
    let table_name = extract_luau_identifier_name(table, source)?;
    if !module_names.contains(table_name) {
        return None;
    }

    let field_name = match separator {
        "." => node.child_by_field_name("field")?,
        ":" => node.child_by_field_name("method")?,
        _ => unreachable!(),
    };
    let field_name = node_text(field_name, source).map(str::trim)?;
    if field_name.is_empty() {
        return None;
    }

    Some(format!("{table_name}{separator}{field_name}"))
}

fn unwrap_luau_variable(node: Node<'_>) -> Node<'_> {
    let mut current = node;
    loop {
        if current.kind() != "variable" {
            return current;
        }

        let Some(child) = named_children(current).into_iter().next() else {
            return current;
        };
        current = child;
    }
}

fn extract_luau_assigned_function_signature(
    target: &str,
    function_definition: Node<'_>,
    source: &str,
) -> Option<String> {
    let suffix = extract_tree_sitter_prefix_signature(function_definition, source, "body")?;
    let suffix = suffix.strip_prefix("function").map(str::trim_start)?;
    let signature = normalize_signature(&format!("function {target}{suffix}"));
    if signature.is_empty() {
        None
    } else {
        Some(signature)
    }
}

fn extract_tree_sitter_export_statement(node: Node<'_>, source: &str) -> Vec<String> {
    let is_default_export = node_text(node, source)
        .map(|text| text.trim_start().starts_with("export default"))
        .unwrap_or(false);

    if let Some(declaration) = node.child_by_field_name("declaration") {
        let signatures = extract_tree_sitter_export_declaration(declaration, source);
        if !signatures.is_empty() {
            return signatures
                .into_iter()
                .map(|signature| {
                    if is_default_export {
                        format!("ts/js:export default {signature}")
                    } else {
                        format!("ts/js:{signature}")
                    }
                })
                .collect();
        }
    }

    if let Some(value) = node.child_by_field_name("value") {
        if let Some(signature) = extract_tree_sitter_default_export_value(value, source) {
            return vec![format!("ts/js:export default {signature}")];
        }
    }

    node_text(node, source)
        .map(normalize_export_statement_signature)
        .filter(|signature| !signature.is_empty())
        .map(|signature| vec![format!("ts/js:{signature}")])
        .unwrap_or_default()
}

fn extract_tree_sitter_export_declaration(node: Node<'_>, source: &str) -> Vec<String> {
    match node.kind() {
        "function_declaration" | "generator_function_declaration" => {
            extract_tree_sitter_function_signature(node, source)
                .into_iter()
                .collect()
        }
        "class_declaration" => extract_tree_sitter_prefix_signature(node, source, "body")
            .into_iter()
            .collect(),
        "interface_declaration" => extract_tree_sitter_prefix_signature(node, source, "body")
            .into_iter()
            .collect(),
        "enum_declaration" => extract_tree_sitter_prefix_signature(node, source, "body")
            .into_iter()
            .collect(),
        "type_alias_declaration" => node_text(node, source)
            .map(normalize_export_statement_signature)
            .filter(|signature| !signature.is_empty())
            .into_iter()
            .collect(),
        "lexical_declaration" => extract_tree_sitter_variable_signatures(node, source),
        "variable_declaration" => extract_tree_sitter_variable_signatures(node, source),
        _ => Vec::new(),
    }
}

fn extract_tree_sitter_default_export_value(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "function" | "function_expression" | "generator_function" => {
            extract_tree_sitter_function_signature(node, source)
        }
        "class" => extract_tree_sitter_prefix_signature(node, source, "body"),
        _ => None,
    }
}

fn extract_tree_sitter_function_signature(node: Node<'_>, source: &str) -> Option<String> {
    let body = node.child_by_field_name("body")?;
    node_text_range(source, node.start_byte(), body.start_byte())
        .map(normalize_export_statement_signature)
        .filter(|signature| !signature.is_empty())
}

fn extract_tree_sitter_prefix_signature(
    node: Node<'_>,
    source: &str,
    field_name: &str,
) -> Option<String> {
    let end = node
        .child_by_field_name(field_name)
        .map(|field| field.start_byte())
        .unwrap_or_else(|| node.end_byte());
    node_text_range(source, node.start_byte(), end)
        .map(normalize_tree_sitter_signature)
        .filter(|signature| !signature.is_empty())
}

fn extract_tree_sitter_prefix_before_named_child(
    node: Node<'_>,
    source: &str,
    stop_kinds: &[&str],
) -> Option<String> {
    let end = named_children(node)
        .into_iter()
        .find(|child| stop_kinds.contains(&child.kind()))
        .map(|child| child.start_byte())
        .unwrap_or_else(|| node.end_byte());
    node_text_range(source, node.start_byte(), end)
        .map(normalize_tree_sitter_signature)
        .filter(|signature| !signature.is_empty())
}

fn extract_tree_sitter_prefix_before_fields(
    node: Node<'_>,
    source: &str,
    stop_fields: &[&str],
) -> Option<String> {
    let end = stop_fields
        .iter()
        .filter_map(|field_name| {
            node.child_by_field_name(field_name)
                .map(|field| field.start_byte())
        })
        .min()
        .unwrap_or_else(|| node.end_byte());

    node_text_range(source, node.start_byte(), end)
        .map(normalize_tree_sitter_signature)
        .filter(|signature| !signature.is_empty())
}

fn extract_tree_sitter_variable_signatures(node: Node<'_>, source: &str) -> Vec<String> {
    let kind = node
        .child_by_field_name("kind")
        .and_then(|child| node_text(child, source))
        .map(str::trim)
        .unwrap_or("var");

    let mut signatures = Vec::new();
    for index in 0..node.named_child_count() {
        let Some(child) = node.named_child(index) else {
            continue;
        };
        if child.kind() != "variable_declarator" {
            continue;
        }

        let Some(name) = child.child_by_field_name("name") else {
            continue;
        };
        if name.kind() != "identifier" {
            continue;
        }

        let Some(name_text) = node_text(name, source).map(str::trim) else {
            continue;
        };
        if name_text.is_empty() {
            continue;
        }

        let mut signature = format!("{kind} {name_text}");
        if let Some(type_annotation) = child
            .child_by_field_name("type")
            .and_then(|annotation| node_text(annotation, source))
            .map(str::trim)
        {
            if !type_annotation.is_empty() {
                signature.push_str(type_annotation);
            }
        }

        let normalized = normalize_signature(&signature);
        if !normalized.is_empty() {
            signatures.push(normalized);
        }
    }

    signatures
}

fn normalize_export_statement_signature(raw: &str) -> String {
    normalize_tree_sitter_signature(raw)
}

fn normalize_tree_sitter_signature(raw: &str) -> String {
    let mut normalized =
        normalize_signature(raw.trim().trim_end_matches(';').trim_end_matches(':'));
    for (from, to) in [(",)", ")"), (", }", " }"), (",]", "]")] {
        normalized = normalized.replace(from, to);
    }
    normalized
}

fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut children = Vec::new();
    for index in 0..node.named_child_count() {
        if let Some(child) = node.named_child(index) {
            children.push(child);
        }
    }
    children
}

fn is_exported_tree_sitter_name(node: Node<'_>, source: &str) -> bool {
    node.child_by_field_name("name")
        .and_then(|child| node_text(child, source))
        .map(|name| {
            name.chars()
                .next()
                .map(|first| first.is_ascii_uppercase())
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn is_public_python_name(node: Node<'_>, source: &str) -> bool {
    node.child_by_field_name("name")
        .and_then(|child| node_text(child, source))
        .map(|name| !name.starts_with('_'))
        .unwrap_or(false)
}

fn has_tree_sitter_keyword(
    node: Node<'_>,
    source: &str,
    stop_field_name: &str,
    keyword: &str,
) -> bool {
    extract_tree_sitter_prefix_signature(node, source, stop_field_name)
        .map(|signature| contains_signature_keyword(&signature, keyword))
        .unwrap_or(false)
}

fn normalize_kotlin_signature(signature: &str) -> Option<String> {
    if contains_signature_keyword(signature, "private")
        || contains_signature_keyword(signature, "protected")
        || contains_signature_keyword(signature, "internal")
    {
        return None;
    }

    trim_signature_to_keywords(
        signature,
        &[
            "public",
            "data",
            "sealed",
            "enum",
            "annotation",
            "value",
            "suspend",
            "class",
            "interface",
            "object",
            "fun",
        ],
    )
}

fn normalize_php_type_signature(signature: &str) -> Option<String> {
    trim_signature_to_keywords(
        signature,
        &[
            "abstract",
            "final",
            "readonly",
            "class",
            "interface",
            "trait",
            "enum",
        ],
    )
}

fn normalize_php_function_signature(signature: &str) -> Option<String> {
    trim_signature_to_keywords(signature, &["function"])
}

fn normalize_php_method_signature(signature: &str, inside_interface: bool) -> Option<String> {
    if contains_any_signature_keyword(signature, &["private", "protected"]) {
        return None;
    }

    if inside_interface {
        return trim_signature_to_keywords(signature, &["function"]);
    }

    if contains_signature_keyword(signature, "public") {
        return trim_signature_to_keywords(signature, &["public"]);
    }

    None
}

fn extract_tree_sitter_php_property_signatures(
    node: Node<'_>,
    source: &str,
    inside_interface: bool,
) -> Vec<String> {
    let visibility = php_visibility_modifier(node, source).or_else(|| {
        if php_has_modifier(node, "var_modifier") || inside_interface {
            Some("public".to_string())
        } else {
            None
        }
    });
    if visibility.as_deref() != Some("public") {
        return Vec::new();
    }

    let mut prefix_parts = vec![visibility.expect("public visibility already checked")];
    if php_has_modifier(node, "static_modifier") {
        prefix_parts.push("static".to_string());
    }
    if php_has_modifier(node, "readonly_modifier") {
        prefix_parts.push("readonly".to_string());
    }
    if let Some(type_annotation) = node
        .child_by_field_name("type")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
    {
        prefix_parts.push(type_annotation);
    }

    let prefix = prefix_parts.join(" ");
    let mut signatures = Vec::new();
    for child in named_children(node) {
        if child.kind() != "property_element" {
            continue;
        }

        let Some(name) = child
            .child_by_field_name("name")
            .and_then(|name| node_text(name, source))
            .map(str::trim)
        else {
            continue;
        };
        if name.is_empty() {
            continue;
        }

        let signature = normalize_signature(&format!("{prefix} {name}"));
        if !signature.is_empty() {
            signatures.push(format!("php:{signature}"));
        }
    }

    signatures
}

fn extract_tree_sitter_php_const_signatures(
    node: Node<'_>,
    source: &str,
    inside_interface: bool,
) -> Vec<String> {
    let visibility = php_visibility_modifier(node, source).or_else(|| {
        if inside_interface {
            Some("public".to_string())
        } else {
            None
        }
    });
    if matches!(visibility.as_deref(), Some("private" | "protected")) {
        return Vec::new();
    }

    let mut prefix_parts = Vec::new();
    if let Some(visibility) = visibility {
        prefix_parts.push(visibility);
    }
    if php_has_modifier(node, "final_modifier") {
        prefix_parts.push("final".to_string());
    }
    prefix_parts.push("const".to_string());
    if let Some(type_annotation) = node
        .child_by_field_name("type")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
    {
        prefix_parts.push(type_annotation);
    }

    let prefix = prefix_parts.join(" ");
    let mut signatures = Vec::new();
    for child in named_children(node) {
        if child.kind() != "const_element" {
            continue;
        }

        let Some(name) = named_children(child)
            .into_iter()
            .find(|named_child| named_child.kind() == "name")
            .and_then(|name| node_text(name, source))
            .map(str::trim)
        else {
            continue;
        };
        if name.is_empty() {
            continue;
        }

        let signature = normalize_signature(&format!("{prefix} {name}"));
        if !signature.is_empty() {
            signatures.push(format!("php:{signature}"));
        }
    }

    signatures
}

fn php_visibility_modifier(node: Node<'_>, source: &str) -> Option<String> {
    named_children(node)
        .into_iter()
        .find(|child| child.kind() == "visibility_modifier")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
}

fn php_has_modifier(node: Node<'_>, modifier_kind: &str) -> bool {
    named_children(node)
        .into_iter()
        .any(|child| child.kind() == modifier_kind)
}

fn normalize_swift_signature(signature: &str) -> Option<String> {
    if !contains_any_signature_keyword(signature, &["public", "open"]) {
        return None;
    }

    trim_signature_to_keywords(signature, &["public", "open"])
}

fn normalize_swift_protocol_member_signature(
    signature: &str,
    inherited_visibility: Option<&str>,
) -> Option<String> {
    if let Some(signature) = normalize_swift_signature(signature) {
        return Some(signature);
    }

    let inherited_visibility = inherited_visibility?;
    let normalized = normalize_tree_sitter_signature(signature);
    if normalized.is_empty() {
        return None;
    }

    Some(format!("{inherited_visibility} {normalized}"))
}

fn swift_visibility_keyword(signature: &str) -> Option<&'static str> {
    if contains_signature_keyword(signature, "open") {
        Some("open")
    } else if contains_signature_keyword(signature, "public") {
        Some("public")
    } else {
        None
    }
}

fn trim_signature_to_keywords(signature: &str, keywords: &[&str]) -> Option<String> {
    let start = keywords
        .iter()
        .filter_map(|keyword| find_signature_keyword(signature, keyword))
        .min()
        .unwrap_or(0);
    let trimmed = normalize_tree_sitter_signature(signature.get(start..)?.trim());
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn find_signature_keyword(signature: &str, keyword: &str) -> Option<usize> {
    let bytes = signature.as_bytes();
    let keyword_bytes = keyword.as_bytes();
    if keyword_bytes.is_empty() || keyword_bytes.len() > bytes.len() {
        return None;
    }

    for index in 0..=bytes.len() - keyword_bytes.len() {
        if &bytes[index..index + keyword_bytes.len()] != keyword_bytes {
            continue;
        }

        let left_ok = index == 0 || !is_signature_word_byte(bytes[index - 1]);
        let right_index = index + keyword_bytes.len();
        let right_ok = right_index == bytes.len() || !is_signature_word_byte(bytes[right_index]);
        if left_ok && right_ok {
            return Some(index);
        }
    }

    None
}

fn contains_signature_keyword(signature: &str, keyword: &str) -> bool {
    find_signature_keyword(signature, keyword).is_some()
}

fn contains_any_signature_keyword(signature: &str, keywords: &[&str]) -> bool {
    keywords
        .iter()
        .any(|keyword| contains_signature_keyword(signature, keyword))
}

fn is_signature_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    node_text_range(source, node.start_byte(), node.end_byte())
}

fn node_text_range(source: &str, start: usize, end: usize) -> Option<&str> {
    source.get(start..end)
}

fn extract_regex_public_api(
    language: SourceLanguage,
    source: &str,
    regex: &ApiRegexPack,
) -> Vec<String> {
    let mut signatures = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || is_comment_line(trimmed, language.extension_label()) {
            continue;
        }

        match language {
            SourceLanguage::TypeScript
            | SourceLanguage::Tsx
            | SourceLanguage::JavaScript
            | SourceLanguage::Jsx
            | SourceLanguage::Php
            | SourceLanguage::Swift
            | SourceLanguage::Luau => {}
            SourceLanguage::Go => {
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
            SourceLanguage::Python => {
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
            SourceLanguage::Java => {
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
            SourceLanguage::Kotlin => {
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
            SourceLanguage::Csharp => {
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
            SourceLanguage::Rust => {}
        }
    }

    signatures.sort();
    signatures.dedup();
    signatures
}

fn lex_source(source: &str, language: SourceLanguage) -> LexedSource {
    match language.lex_strategy() {
        LexStrategy::CStyle => lex_c_style_source(source, language),
        LexStrategy::Python => lex_python_source(source),
        LexStrategy::Php => lex_php_source(source),
        LexStrategy::Luau => lex_luau_source(source),
    }
}

fn lex_c_style_source(source: &str, language: SourceLanguage) -> LexedSource {
    let bytes = source.as_bytes();
    let mut masked = bytes.to_vec();
    let mut comments = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if language == SourceLanguage::Rust {
            if let Some(end) = skip_rust_raw_string(bytes, i) {
                mask_range(&mut masked, bytes, i, end);
                i = end;
                continue;
            }
        }

        if language == SourceLanguage::Swift {
            if let Some(end) = skip_swift_string(bytes, i) {
                mask_range(&mut masked, bytes, i, end);
                i = end;
                continue;
            }
        }

        if uses_backtick_strings(language) && bytes[i] == b'`' {
            let end = skip_backtick_string(bytes, i);
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if uses_single_quote_strings(language) && bytes[i] == b'\'' {
            let end = skip_quoted_string(bytes, i, b'\'');
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if is_csharp_verbatim_string_start(bytes, i) {
            let end = skip_csharp_verbatim_string(bytes, i);
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'"' {
            let end = skip_quoted_string(bytes, i, b'"');
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'/' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'/' {
                let end = skip_line_comment(bytes, i);
                comments.push(source[i + 2..end].to_string());
                mask_range(&mut masked, bytes, i, end);
                i = end;
                continue;
            }

            if bytes[i + 1] == b'*' {
                let end = skip_block_comment(bytes, i, language.uses_nested_block_comments());
                let content_end = if end >= i + 4 { end - 2 } else { end };
                comments.push(source[i + 2..content_end].to_string());
                mask_range(&mut masked, bytes, i, end);
                i = end;
                continue;
            }
        }

        i += 1;
    }

    LexedSource {
        comments,
        masked_code: String::from_utf8(masked).unwrap_or_else(|_| source.to_string()),
    }
}

fn lex_php_source(source: &str) -> LexedSource {
    let bytes = source.as_bytes();
    let mut masked = bytes.to_vec();
    let mut comments = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if is_csharp_verbatim_string_start(bytes, i) {
            let end = skip_csharp_verbatim_string(bytes, i);
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'`' {
            let end = skip_backtick_string(bytes, i);
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if uses_single_quote_strings(SourceLanguage::Php) && (bytes[i] == b'\'' || bytes[i] == b'"')
        {
            let end = skip_quoted_string(bytes, i, bytes[i]);
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'#' {
            if matches!(bytes.get(i + 1), Some(b'[')) {
                i += 1;
                continue;
            }

            let end = skip_line_comment(bytes, i);
            comments.push(source[i + 1..end].to_string());
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'/' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'/' {
                let end = skip_line_comment(bytes, i);
                comments.push(source[i + 2..end].to_string());
                mask_range(&mut masked, bytes, i, end);
                i = end;
                continue;
            }

            if bytes[i + 1] == b'*' {
                let end = skip_block_comment(bytes, i, false);
                let content_end = if end >= i + 4 { end - 2 } else { end };
                comments.push(source[i + 2..content_end].to_string());
                mask_range(&mut masked, bytes, i, end);
                i = end;
                continue;
            }
        }

        i += 1;
    }

    LexedSource {
        comments,
        masked_code: String::from_utf8(masked).unwrap_or_else(|_| source.to_string()),
    }
}

fn lex_luau_source(source: &str) -> LexedSource {
    let bytes = source.as_bytes();
    let mut masked = bytes.to_vec();
    let mut comments = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if let Some((_, _, end)) = skip_luau_long_bracket(bytes, i) {
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'\'' || bytes[i] == b'"' {
            let end = skip_quoted_string(bytes, i, bytes[i]);
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            if let Some((content_start, content_end, end)) = skip_luau_long_bracket(bytes, i + 2) {
                comments.push(source[content_start..content_end].to_string());
                mask_range(&mut masked, bytes, i, end);
                i = end;
                continue;
            }

            let end = skip_line_comment(bytes, i);
            comments.push(source[i + 2..end].to_string());
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        i += 1;
    }

    LexedSource {
        comments,
        masked_code: String::from_utf8(masked).unwrap_or_else(|_| source.to_string()),
    }
}

fn lex_python_source(source: &str) -> LexedSource {
    let bytes = source.as_bytes();
    let mut masked = bytes.to_vec();
    let mut comments = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if let Some(end) = skip_python_triple_quoted_string(bytes, i) {
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'\'' || bytes[i] == b'"' {
            let end = skip_quoted_string(bytes, i, bytes[i]);
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'#' {
            let end = skip_line_comment(bytes, i);
            comments.push(source[i + 1..end].to_string());
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        i += 1;
    }

    LexedSource {
        comments,
        masked_code: String::from_utf8(masked).unwrap_or_else(|_| source.to_string()),
    }
}

fn uses_backtick_strings(language: SourceLanguage) -> bool {
    matches!(
        language,
        SourceLanguage::TypeScript
            | SourceLanguage::Tsx
            | SourceLanguage::JavaScript
            | SourceLanguage::Jsx
            | SourceLanguage::Go
    )
}

fn uses_single_quote_strings(language: SourceLanguage) -> bool {
    !matches!(language, SourceLanguage::Rust)
}

fn mask_range(masked: &mut [u8], original: &[u8], start: usize, end: usize) {
    for index in start..end.min(masked.len()) {
        masked[index] = match original[index] {
            b'\n' | b'\r' | b'\t' | b' ' => original[index],
            _ => b' ',
        };
    }
}

fn skip_line_comment(bytes: &[u8], start: usize) -> usize {
    let mut index = start;
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    index
}

fn skip_block_comment(bytes: &[u8], start: usize, allow_nested: bool) -> usize {
    let mut index = start + 2;
    let mut depth = 1usize;

    while index + 1 < bytes.len() {
        if allow_nested && bytes[index] == b'/' && bytes[index + 1] == b'*' {
            depth += 1;
            index += 2;
            continue;
        }

        if bytes[index] == b'*' && bytes[index + 1] == b'/' {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return index;
            }
            continue;
        }

        index += 1;
    }

    bytes.len()
}

fn skip_quoted_string(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut index = start + 1;

    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
            continue;
        }

        if bytes[index] == quote {
            return index + 1;
        }

        index += 1;
    }

    bytes.len()
}

fn skip_swift_string(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start;
    let mut hashes = 0usize;

    while index < bytes.len() && bytes[index] == b'#' {
        hashes += 1;
        index += 1;
    }

    if index >= bytes.len() || bytes[index] != b'"' {
        return None;
    }

    let multiline = matches!(bytes.get(index..index + 3), Some(prefix) if prefix == b"\"\"\"");
    index += if multiline { 3 } else { 1 };

    while index < bytes.len() {
        if !multiline && bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
            continue;
        }

        if multiline {
            if !matches!(bytes.get(index..index + 3), Some(prefix) if prefix == b"\"\"\"") {
                index += 1;
                continue;
            }

            let end = index + 3;
            if matches_hash_suffix(bytes, end, hashes) {
                return Some((end + hashes).min(bytes.len()));
            }

            index += 1;
            continue;
        }

        if bytes[index] == b'"' && matches_hash_suffix(bytes, index + 1, hashes) {
            return Some((index + 1 + hashes).min(bytes.len()));
        }

        index += 1;
    }

    Some(bytes.len())
}

fn skip_backtick_string(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 1;

    while index < bytes.len() {
        if bytes[index] == b'`' {
            return index + 1;
        }
        index += 1;
    }

    bytes.len()
}

fn skip_python_triple_quoted_string(bytes: &[u8], start: usize) -> Option<usize> {
    if start + 2 >= bytes.len() {
        return None;
    }

    let quote = bytes[start];
    if (quote == b'\'' || quote == b'"') && bytes[start + 1] == quote && bytes[start + 2] == quote {
        let mut index = start + 3;
        while index + 2 < bytes.len() {
            if bytes[index] == quote && bytes[index + 1] == quote && bytes[index + 2] == quote {
                return Some(index + 3);
            }
            index += 1;
        }
        return Some(bytes.len());
    }

    None
}

fn skip_luau_long_bracket(bytes: &[u8], start: usize) -> Option<(usize, usize, usize)> {
    if bytes.get(start) != Some(&b'[') {
        return None;
    }

    let mut index = start + 1;
    let mut equals_count = 0usize;
    while index < bytes.len() && bytes[index] == b'=' {
        equals_count += 1;
        index += 1;
    }

    if bytes.get(index) != Some(&b'[') {
        return None;
    }

    let content_start = index + 1;
    let mut cursor = content_start;
    while cursor < bytes.len() {
        if bytes[cursor] != b']' {
            cursor += 1;
            continue;
        }

        let mut end = cursor + 1;
        let mut matched = 0usize;
        while matched < equals_count && end < bytes.len() && bytes[end] == b'=' {
            matched += 1;
            end += 1;
        }

        if matched == equals_count && bytes.get(end) == Some(&b']') {
            return Some((content_start, cursor, end + 1));
        }

        cursor += 1;
    }

    Some((content_start, bytes.len(), bytes.len()))
}

fn matches_hash_suffix(bytes: &[u8], start: usize, hashes: usize) -> bool {
    start + hashes <= bytes.len()
        && bytes[start..start + hashes]
            .iter()
            .all(|byte| *byte == b'#')
}

fn skip_rust_raw_string(bytes: &[u8], start: usize) -> Option<usize> {
    if !matches!(bytes[start], b'r' | b'b') {
        return None;
    }

    if start > 0 && is_identifier_continue(bytes[start - 1]) {
        return None;
    }

    let mut index = start;
    if bytes[index] == b'b' {
        index += 1;
        if index >= bytes.len() || bytes[index] != b'r' {
            return None;
        }
    }

    if bytes[index] != b'r' {
        return None;
    }
    index += 1;

    let mut hashes = 0usize;
    while index < bytes.len() && bytes[index] == b'#' {
        hashes += 1;
        index += 1;
    }

    if index >= bytes.len() || bytes[index] != b'"' {
        return None;
    }
    index += 1;

    while index < bytes.len() {
        if bytes[index] == b'"' {
            let mut matched = true;
            for offset in 0..hashes {
                if index + 1 + offset >= bytes.len() || bytes[index + 1 + offset] != b'#' {
                    matched = false;
                    break;
                }
            }
            if matched {
                return Some((index + 1 + hashes).min(bytes.len()));
            }
        }
        index += 1;
    }

    Some(bytes.len())
}

fn is_csharp_verbatim_string_start(bytes: &[u8], start: usize) -> bool {
    matches!(
        bytes.get(start..start + 2),
        Some(prefix) if prefix == b"@\""
    ) || matches!(bytes.get(start..start + 3), Some(prefix) if prefix == b"$@\"" || prefix == b"@$\"")
}

fn skip_csharp_verbatim_string(bytes: &[u8], start: usize) -> usize {
    let quote_start = if matches!(bytes.get(start..start + 3), Some(prefix) if prefix == b"$@\"" || prefix == b"@$\"")
    {
        start + 3
    } else {
        start + 2
    };

    let mut index = quote_start;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            if index + 1 < bytes.len() && bytes[index + 1] == b'"' {
                index += 2;
                continue;
            }
            return index + 1;
        }
        index += 1;
    }

    bytes.len()
}

fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
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
                let sig = format_rust_signature(&item_fn.sig);
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
                        let sig = format_rust_signature(&method.sig);
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

                        let sig = format_rust_signature(&method.sig);
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

fn format_rust_signature(signature: &Signature) -> String {
    let qualifiers = format_rust_qualifiers(signature);
    let generics = format_rust_generics(signature);
    let params = signature
        .inputs
        .iter()
        .map(format_rust_fn_arg)
        .collect::<Vec<_>>()
        .join(", ");
    let output = format_rust_return_type(&signature.output);
    let where_clause = signature
        .generics
        .where_clause
        .as_ref()
        .map(|clause| {
            let normalized = normalize_signature(&clause.to_token_stream().to_string());
            format!(" {}", normalized.trim_end_matches(','))
        })
        .unwrap_or_default();

    format!(
        "{qualifiers}{}{generics}({params}){output}{where_clause}",
        signature.ident
    )
}

fn format_rust_qualifiers(signature: &Signature) -> String {
    let mut qualifiers = Vec::new();
    if signature.constness.is_some() {
        qualifiers.push("const".to_string());
    }
    if signature.asyncness.is_some() {
        qualifiers.push("async".to_string());
    }
    if signature.unsafety.is_some() {
        qualifiers.push("unsafe".to_string());
    }
    if let Some(abi) = &signature.abi {
        qualifiers.push(normalize_signature(&abi.to_token_stream().to_string()));
    }

    if qualifiers.is_empty() {
        String::new()
    } else {
        format!("{} ", qualifiers.join(" "))
    }
}

fn format_rust_generics(signature: &Signature) -> String {
    let params = normalize_signature(&signature.generics.params.to_token_stream().to_string());
    if params.is_empty() {
        String::new()
    } else {
        format!("<{params}>")
    }
}

fn format_rust_fn_arg(argument: &FnArg) -> String {
    match argument {
        FnArg::Receiver(receiver) => {
            if receiver.colon_token.is_some() {
                let mut rendered = String::new();
                if receiver.mutability.is_some() {
                    rendered.push_str("mut ");
                }
                rendered.push_str("self: ");
                rendered.push_str(&normalize_signature(
                    &receiver.ty.to_token_stream().to_string(),
                ));
                return rendered;
            }

            let mut rendered = String::new();
            if let Some((_, lifetime)) = &receiver.reference {
                rendered.push('&');
                if let Some(lifetime) = lifetime {
                    rendered.push_str(&lifetime.to_token_stream().to_string());
                    rendered.push(' ');
                }
            }
            if receiver.mutability.is_some() {
                rendered.push_str("mut ");
            }
            rendered.push_str("self");
            rendered
        }
        FnArg::Typed(argument) => {
            let pattern = normalize_signature(&argument.pat.to_token_stream().to_string());
            let ty = normalize_signature(&argument.ty.to_token_stream().to_string());
            format!("{pattern}: {ty}")
        }
    }
}

fn format_rust_return_type(output: &ReturnType) -> String {
    match output {
        ReturnType::Default => String::new(),
        ReturnType::Type(_, ty) => {
            format!(
                " -> {}",
                normalize_signature(&ty.to_token_stream().to_string())
            )
        }
    }
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
        ("< ", "<"),
        (" >", ">"),
        (" = ", "="),
        ("& mut ", "&mut "),
        ("& ", "&"),
        (" & ", "&"),
    ] {
        normalized = normalized.replace(from, to);
    }
    normalized
}

fn is_comment_line(line: &str, extension: &str) -> bool {
    match extension {
        "py" => line.starts_with('#'),
        "php" => line.starts_with('#') || line.starts_with("//") || line.starts_with("/*"),
        "luau" => line.starts_with("--"),
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

    use crate::mvs::manifest::ScanPolicy;

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

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

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

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "rust:fn handshake(version: u32) -> bool"));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "rust:impl-fn HostAdapter::connect(&self, target: &str) -> bool"
        }));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("private_method")));
    }

    #[test]
    fn rust_ast_extraction_preserves_generics_and_where_clauses_in_canonical_form() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("lib.rs"),
            r#"
            pub async fn load<'a, T>(value: &'a T) -> &'a T
            where
                T: Clone,
            {
                value
            }
        "#,
        )
        .expect("failed to write rust fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "rust:fn async load<'a, T>(value: &'a T) -> &'a T where T: Clone"
        }));
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

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.is_empty());
        assert!(report.public_api.is_empty());
    }

    #[test]
    fn ignores_tags_inside_rust_raw_strings() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("lib.rs"),
            r##"
            const EXAMPLE: &str = r#"
            // @mvs-feature("fake_feature")
            // @mvs-protocol("fake_protocol")
            "#;

            /// @mvs-feature("real_feature")
            /// @mvs-protocol("real_protocol")
            pub fn handshake(version: u32) -> bool { version > 0 }
        "##,
        )
        .expect("failed to write rust fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.contains("real_feature"));
        assert!(report.protocol_tags.contains("real_protocol"));
        assert!(!report.feature_tags.contains("fake_feature"));
        assert!(!report.protocol_tags.contains("fake_protocol"));
    }

    #[test]
    fn captures_tags_from_block_comments() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("billing.ts"),
            r#"
            /*
             * @mvs-feature("billing")
             * @mvs-protocol("billing-api-v1")
             */
            export function checkout(amount: number): number {
              return amount
            }
        "#,
        )
        .expect("failed to write ts fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.contains("billing"));
        assert!(report.protocol_tags.contains("billing-api-v1"));
    }

    #[test]
    fn tree_sitter_captures_named_exports_and_reexports() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.ts"),
            r#"
            const login = (username: string): string => username;
            function logout(): void {}

            export {
              login,
              logout as signOut,
            };

            export { login as authenticate } from "./auth";
            export * as authApi from "./auth";
        "#,
        )
        .expect("failed to write ts fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| { entry.signature == "ts/js:export { login, logout as signOut }" }));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "ts/js:export { login as authenticate } from \"./auth\""
        }));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:export * as authApi from \"./auth\""));
    }

    #[test]
    fn tree_sitter_captures_default_export_without_body_noise() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.ts"),
            r#"
            export default function login(
              username: string,
              password: string,
            ): Promise<string> {
              return Promise.resolve(`${username}:${password}`);
            }
        "#,
        )
        .expect("failed to write ts fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.public_api.iter().any(|entry| {
            entry.signature
                == "ts/js:export default function login(username: string, password: string): Promise<string>"
        }));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("Promise.resolve")));
    }

    #[test]
    fn tree_sitter_captures_go_exported_functions_and_methods() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.go"),
            r#"
            package demo

            type service struct{}

            func ExportedLogin(username string) string {
                return username
            }

            func unexported() {}

            func (s *service) hidden(target string) error {
                return nil
            }

            func (s *service) Connect(target string) error {
                return nil
            }
        "#,
        )
        .expect("failed to write go fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:func ExportedLogin(username string) string"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:func(s *service) Connect(target string) error"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("unexported")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("hidden")));
    }

    #[test]
    fn tree_sitter_captures_python_public_defs_without_decorators_or_nested_locals() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.py"),
            r#"
            class Worker:
                @staticmethod
                def run_job(name: str) -> str:
                    def helper() -> str:
                        return name
                    return helper()

                def _hidden(self) -> str:
                    return "hidden"

            @decorator
            def login(username: str) -> str:
                return username

            def _internal() -> str:
                return "hidden"
        "#,
        )
        .expect("failed to write python fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:def run_job(name: str) -> str"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:def login(username: str) -> str"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("helper")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("@decorator")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("_internal")));
    }

    #[test]
    fn tree_sitter_captures_java_public_types_and_methods() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("AuthApi.java"),
            r#"
            package demo;

            @Deprecated
            public record Session(String token) {}

            public class AuthApi {
                @Deprecated
                public String login(String username) {
                    return username;
                }

                void hidden() {}

                public static class Nested {}
            }
        "#,
        )
        .expect("failed to write java fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "java:type public record Session(String token)"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "java:type public class AuthApi"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "java:type public static class Nested"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "java:method public String login(String username)"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("@Deprecated")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("hidden")));
    }

    #[test]
    fn tree_sitter_captures_kotlin_public_declarations() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.kt"),
            r#"
            public class AuthApi {
                fun login(username: String): String {
                    return username
                }

                private fun hidden(): String {
                    return "hidden"
                }

                data class Session(val token: String)
            }

            internal object InternalDefaults

            suspend fun load(token: String): String {
                return token
            }

            object Defaults
        "#,
        )
        .expect("failed to write kotlin fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "kotlin:public class AuthApi"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "kotlin:fun login(username: String): String"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "kotlin:data class Session(val token: String)"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "kotlin:suspend fun load(token: String): String"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "kotlin:object Defaults"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("hidden")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("InternalDefaults")));
    }

    #[test]
    fn tree_sitter_captures_csharp_public_types_and_methods_without_attributes() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("Api.cs"),
            r#"
            namespace Demo;

            [Obsolete]
            public record Session(string Token);

            public class AuthApi {
                [Obsolete]
                public static string Login(string username) {
                    return username;
                }

                private static string Hidden(string username) {
                    return username;
                }

                public struct Result { }
            }
        "#,
        )
        .expect("failed to write csharp fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "csharp:type public record Session(string Token)"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "csharp:type public class AuthApi"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "csharp:type public struct Result"));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "csharp:method public static string Login(string username)"
        }));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("[Obsolete]")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("Hidden")));
    }

    #[test]
    fn tree_sitter_captures_swift_public_types_properties_and_protocol_requirements() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("Api.swift"),
            r#"
            let example = """
            // @mvs-feature("fake_feature")
            """

            // @mvs-feature("swift_bridge")
            // @mvs-protocol("swift-api-v1")
            public struct Session {}

            public protocol SessionContract {
                var token: String { get }
                func renew(target: String) -> Bool
            }

            public class AuthApi {
                public var status: String {
                    "ready"
                }

                public func login(username: String) -> String {
                    username
                }

                private var hiddenStatus: String {
                    "hidden"
                }

                private func hidden() -> String {
                    "hidden"
                }
            }
        "#,
        )
        .expect("failed to write swift fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.contains("swift_bridge"));
        assert!(report.protocol_tags.contains("swift-api-v1"));
        assert!(!report.feature_tags.contains("fake_feature"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "swift:public struct Session"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "swift:public protocol SessionContract"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| { entry.signature == "swift:public var token: String { get }" }));
        assert!(report
            .public_api
            .iter()
            .any(|entry| { entry.signature == "swift:public func renew(target: String) -> Bool" }));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "swift:public class AuthApi"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "swift:public var status: String"));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "swift:public func login(username: String) -> String"
        }));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("hidden")));
    }

    #[test]
    fn tree_sitter_captures_php_public_api_constants_properties_and_hash_comments() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("Api.php"),
            r#"
            <?php

            # @mvs-feature("php_jobs")
            /* @mvs-protocol("php-worker-v1") */
            #[Deprecated]
            function login(string $username): string {
                return $username;
            }

            const VERSION = "1";

            #[Deprecated]
            final class AuthApi {
                public const STATUS_READY = "ready";
                public readonly string $token;
                public static string $sharedName;
                private string $secret;

                #[Deprecated]
                public static function run(string $name): string {
                    return $name;
                }

                private function hidden(string $name): string {
                    return $name;
                }
            }

            interface Contract {
                const SYNC = "sync";
                public function sync(string $token): void;
            }
        "#,
        )
        .expect("failed to write php fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.contains("php_jobs"));
        assert!(report.protocol_tags.contains("php-worker-v1"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:function login(string $username): string"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:const VERSION"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:final class AuthApi"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:public const STATUS_READY"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:public readonly string $token"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:public static string $sharedName"));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "php:public static function run(string $name): string"
        }));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:interface Contract"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:public const SYNC"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:function sync(string $token): void"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("#[Deprecated]")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("hidden")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("$secret")));
    }

    #[test]
    fn tree_sitter_captures_luau_global_functions_module_exports_and_export_types() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("Api.luau"),
            r#"
            local fixture = [[
            -- @mvs-feature("fake_feature")
            ]]

            -- @mvs-feature("luau_bridge")
            --[[ @mvs-protocol("luau-module-v1") ]]
            export type Session = {
                token: string,
            }

            function connect(target: string): boolean
                return target ~= ""
            end

            local Api = {
                ping = function(target: string): boolean
                    return target ~= ""
                end,
                version = "v1",
            }

            Api.connect = function(target: string): boolean
                return target ~= ""
            end

            function Api:refresh(token: string): boolean
                return token ~= ""
            end

            local Internal = {}

            function Internal.hidden(): boolean
                return false
            end

            local function hidden(): boolean
                return false
            end

            return Api
        "#,
        )
        .expect("failed to write luau fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.contains("luau_bridge"));
        assert!(report.protocol_tags.contains("luau-module-v1"));
        assert!(!report.feature_tags.contains("fake_feature"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "luau:export type Session={ token: string }"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "luau:function Api.ping(target: string): boolean"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "luau:field Api.version"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "luau:function Api.connect(target: string): boolean"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "luau:function Api:refresh(token: string): boolean"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "luau:function connect(target: string): boolean"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("hidden")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("Internal.hidden")));
    }

    #[test]
    fn public_api_roots_scope_api_inventory_without_hiding_tags() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.ts"),
            r#"
            // @mvs-feature("auth")
            // @mvs-protocol("auth-api")
            export function login(username: string): Promise<string> {
              return Promise.resolve(username)
            }
        "#,
        )
        .expect("failed to write api fixture");

        fs::write(
            src.join("internal.ts"),
            r#"
            // @mvs-feature("background_jobs")
            // @mvs-protocol("jobs-api")
            export function runJob(name: string): string {
              return name
            }
        "#,
        )
        .expect("failed to write internal fixture");

        let report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/api.ts".to_string()],
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(report.feature_tags.contains("auth"));
        assert!(report.feature_tags.contains("background_jobs"));
        assert!(report.protocol_tags.contains("auth-api"));
        assert!(report.protocol_tags.contains("jobs-api"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("login")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("runJob")));
    }

    #[test]
    fn exclude_paths_skip_tag_and_api_scans() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(src.join("generated")).expect("failed to create generated dir");

        fs::write(
            src.join("generated/client.ts"),
            r#"
            // @mvs-feature("generated_api")
            // @mvs-protocol("generated-api")
            export function request(path: string): string {
              return path
            }
        "#,
        )
        .expect("failed to write generated fixture");

        let report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: vec!["src/generated".to_string()],
                public_api_roots: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(report.feature_tags.is_empty());
        assert!(report.protocol_tags.is_empty());
        assert!(report.public_api.is_empty());
    }

    #[test]
    fn ignores_exports_inside_template_strings() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.ts"),
            r#"
            const fixture = `
            export function fakeLogin(username: string): Promise<string> {
              return Promise.resolve(username)
            }
            `;

            export function realLogin(username: string): Promise<string> {
              return Promise.resolve(username)
            }
        "#,
        )
        .expect("failed to write ts fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("realLogin")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("fakeLogin")));
    }

    #[test]
    fn public_api_item_filters_scope_signatures_within_selected_root() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.ts"),
            r#"
            // @mvs-feature("auth")
            // @mvs-protocol("auth-api")
            export function login(username: string, password: string): Promise<string> {
              return Promise.resolve(`${username}:${password}`)
            }

            export interface Session {
              token: string
            }

            export const buildSession = (token: string): Session => ({ token })
        "#,
        )
        .expect("failed to write api fixture");

        let report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/api.ts".to_string()],
                public_api_includes: vec!["ts/js:function login*".to_string()],
                public_api_excludes: vec!["ts/js:const buildSession".to_string()],
            },
        )
        .expect("crawler failed");

        assert!(report.feature_tags.contains("auth"));
        assert!(report.protocol_tags.contains("auth-api"));
        assert_eq!(report.public_api.len(), 1);
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature.starts_with("ts/js:function login")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("Session")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("buildSession")));
    }
}
