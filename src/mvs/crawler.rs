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
}

#[derive(Debug, Clone)]
struct LexedSource {
    comments: Vec<String>,
    masked_code: String,
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

    extract_regex_public_api(language, masked_code, regex)
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
            | SourceLanguage::Jsx => {
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
    match language {
        SourceLanguage::Python => lex_python_source(source),
        _ => lex_c_style_source(source, language),
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
                let end = skip_block_comment(bytes, i, language == SourceLanguage::Rust);
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
