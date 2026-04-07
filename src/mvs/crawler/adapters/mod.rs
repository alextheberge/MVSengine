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

use std::path::Path;

use tree_sitter::Node;

use super::language::SourceLanguage;
use crate::mvs::manifest::{
    GoExportFollowing, LuaExportFollowing, PythonExportFollowing, RubyExportFollowing,
    TsExportFollowing,
};

pub(super) use go::{GoPackageIndex, GoPackageSource};
pub(super) use python::{PythonModuleIndex, PythonModuleSource};
pub(super) use ts_js::{TsModuleIndex, TsModuleSource};

pub(super) struct TreeSitterExtractionContext<'a> {
    pub ts_module_index: Option<&'a TsModuleIndex>,
    pub python_module_index: Option<&'a PythonModuleIndex>,
    pub ruby_export_following: RubyExportFollowing,
    pub lua_export_following: LuaExportFollowing,
}

pub(super) fn build_ts_module_index(
    files: &[TsModuleSource<'_>],
    export_following: TsExportFollowing,
    root: &Path,
) -> TsModuleIndex {
    ts_js::build_module_index(files, export_following, root)
}

pub(super) fn build_go_package_index(
    files: &[GoPackageSource<'_>],
    export_following: GoExportFollowing,
) -> GoPackageIndex {
    go::build_package_index(files, export_following)
}

pub(super) fn build_python_module_index(
    files: &[PythonModuleSource<'_>],
    export_following: PythonExportFollowing,
    module_roots: &[String],
) -> PythonModuleIndex {
    python::build_module_index(files, export_following, module_roots)
}

pub(super) fn extract_tree_sitter_public_api(
    language: SourceLanguage,
    root: Node<'_>,
    source: &str,
    rel_path: &str,
    context: TreeSitterExtractionContext<'_>,
) -> Vec<String> {
    match language {
        SourceLanguage::TypeScript
        | SourceLanguage::Tsx
        | SourceLanguage::JavaScript
        | SourceLanguage::Jsx => ts_js::extract(root, source, rel_path, context.ts_module_index),
        SourceLanguage::Go => go::extract(root, source),
        SourceLanguage::Python => {
            python::extract(root, source, rel_path, context.python_module_index)
        }
        SourceLanguage::Java => java::extract(root, source),
        SourceLanguage::Kotlin => kotlin::extract(root, source),
        SourceLanguage::Csharp => csharp::extract(root, source),
        SourceLanguage::Dart => Vec::new(),
        SourceLanguage::Php => php::extract(root, source),
        SourceLanguage::Ruby => ruby::extract(root, source, context.ruby_export_following),
        SourceLanguage::Swift => swift::extract(root, source),
        SourceLanguage::Lua => lua::extract(root, source, context.lua_export_following),
        SourceLanguage::Luau => luau::extract(root, source, context.lua_export_following),
        SourceLanguage::Rust | SourceLanguage::Liquid => Vec::new(),
    }
}
