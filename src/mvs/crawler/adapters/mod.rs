// SPDX-License-Identifier: AGPL-3.0-only
mod csharp;
mod go;
mod java;
mod kotlin;
mod lua;
mod lua_family;
mod luau;
mod php;
mod python;
mod ruby;
mod swift;
mod ts_js;

use tree_sitter::Node;

use super::language::SourceLanguage;

pub(super) use python::{PythonModuleIndex, PythonModuleSource};

pub(super) fn build_python_module_index(files: &[PythonModuleSource<'_>]) -> PythonModuleIndex {
    python::build_module_index(files)
}

pub(super) fn extract_tree_sitter_public_api(
    language: SourceLanguage,
    root: Node<'_>,
    source: &str,
    rel_path: &str,
    python_module_index: Option<&PythonModuleIndex>,
) -> Vec<String> {
    match language {
        SourceLanguage::TypeScript
        | SourceLanguage::Tsx
        | SourceLanguage::JavaScript
        | SourceLanguage::Jsx => ts_js::extract(root, source),
        SourceLanguage::Go => go::extract(root, source),
        SourceLanguage::Python => python::extract(root, source, rel_path, python_module_index),
        SourceLanguage::Java => java::extract(root, source),
        SourceLanguage::Kotlin => kotlin::extract(root, source),
        SourceLanguage::Csharp => csharp::extract(root, source),
        SourceLanguage::Php => php::extract(root, source),
        SourceLanguage::Ruby => ruby::extract(root, source),
        SourceLanguage::Swift => swift::extract(root, source),
        SourceLanguage::Lua => lua::extract(root, source),
        SourceLanguage::Luau => luau::extract(root, source),
        SourceLanguage::Rust => Vec::new(),
    }
}
