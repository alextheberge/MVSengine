// SPDX-License-Identifier: AGPL-3.0-only
mod csharp;
mod go;
mod java;
mod kotlin;
mod luau;
mod php;
mod python;
mod swift;
mod ts_js;

use tree_sitter::Node;

use super::language::SourceLanguage;

pub(super) fn extract_tree_sitter_public_api(
    language: SourceLanguage,
    root: Node<'_>,
    source: &str,
) -> Vec<String> {
    match language {
        SourceLanguage::TypeScript
        | SourceLanguage::Tsx
        | SourceLanguage::JavaScript
        | SourceLanguage::Jsx => ts_js::extract(root, source),
        SourceLanguage::Go => go::extract(root, source),
        SourceLanguage::Python => python::extract(root, source),
        SourceLanguage::Java => java::extract(root, source),
        SourceLanguage::Kotlin => kotlin::extract(root, source),
        SourceLanguage::Csharp => csharp::extract(root, source),
        SourceLanguage::Php => php::extract(root, source),
        SourceLanguage::Swift => swift::extract(root, source),
        SourceLanguage::Luau => luau::extract(root, source),
        SourceLanguage::Rust => Vec::new(),
    }
}
