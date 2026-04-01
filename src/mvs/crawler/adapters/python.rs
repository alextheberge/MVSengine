// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::{BTreeMap, BTreeSet};

use tree_sitter::Node;

use super::super::{
    children_by_field_name, extract_tree_sitter_prefix_signature, is_public_python_name,
    named_children, node_text, normalize_tree_sitter_signature,
};
use crate::mvs::manifest::PythonExportFollowing;

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub(crate) struct PythonModuleIndex {
    exports_by_module: BTreeMap<String, BTreeSet<String>>,
}

pub(crate) struct PythonModuleSource<'a> {
    pub rel_path: &'a str,
    pub source: &'a str,
}

#[derive(Default)]
struct PythonImportContext {
    imported_export_sets: BTreeMap<String, BTreeSet<String>>,
    imported_module_exports: BTreeMap<String, BTreeSet<String>>,
    wildcard_imports: Vec<PythonWildcardImport>,
}

#[derive(Clone, Copy)]
struct PythonExtractionContext<'a> {
    explicit_exports: Option<&'a BTreeSet<String>>,
    import_context: &'a PythonImportContext,
    top_level_bindings: &'a BTreeSet<String>,
}

#[derive(Clone)]
struct PythonWildcardImport {
    module_name: String,
    exports: BTreeSet<String>,
}

pub(super) fn build_module_index(
    files: &[PythonModuleSource<'_>],
    export_following: PythonExportFollowing,
    module_roots: &[String],
) -> PythonModuleIndex {
    if export_following == PythonExportFollowing::Off {
        return PythonModuleIndex::default();
    }

    let mut index = PythonModuleIndex::default();

    for _ in 0..4 {
        let mut next = PythonModuleIndex::default();

        for file in files {
            let Some(exports) = summarize_python_module_exports(file.source, &index) else {
                continue;
            };
            for module_name in
                python_module_name_candidates(file.rel_path, export_following, module_roots)
            {
                next.exports_by_module
                    .entry(module_name)
                    .or_default()
                    .extend(exports.iter().cloned());
            }
        }

        if next == index {
            break;
        }
        index = next;
    }

    index
}

pub(super) fn extract(
    root: Node<'_>,
    source: &str,
    _rel_path: &str,
    module_index: Option<&PythonModuleIndex>,
) -> Vec<String> {
    let mut signatures = Vec::new();
    let import_context = build_python_import_context(root, source, module_index);
    let explicit_exports = extract_python_explicit_exports(root, source, &import_context);
    let top_level_bindings = collect_python_top_level_bindings(root, source);
    let extraction_context = PythonExtractionContext {
        explicit_exports: explicit_exports.as_ref(),
        import_context: &import_context,
        top_level_bindings: &top_level_bindings,
    };
    collect_public_api(
        root,
        source,
        &mut signatures,
        false,
        &[],
        extraction_context,
    );
    signatures
}

fn collect_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    inside_callable: bool,
    class_namespace: &[String],
    extraction_context: PythonExtractionContext<'_>,
) {
    for child in named_children(node) {
        collect_definition(
            child,
            source,
            signatures,
            inside_callable,
            class_namespace,
            extraction_context,
        );
    }
}

fn collect_definition(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    inside_callable: bool,
    class_namespace: &[String],
    extraction_context: PythonExtractionContext<'_>,
) {
    match node.kind() {
        "function_definition" => {
            if inside_callable {
                return;
            }

            let Some(name) = python_definition_name(node, source) else {
                return;
            };
            if !python_should_include_name(
                &name,
                class_namespace,
                extraction_context.explicit_exports,
                is_public_python_name(node, source),
            ) {
                return;
            }

            if let Some(signature) = extract_tree_sitter_prefix_signature(node, source, "body") {
                signatures.push(format!(
                    "python:{}",
                    qualify_python_function_signature(&signature, class_namespace)
                ));
            }
        }
        "class_definition" => {
            if inside_callable {
                return;
            }

            let Some(name) = python_definition_name(node, source) else {
                return;
            };
            if !python_should_include_name(
                &name,
                class_namespace,
                extraction_context.explicit_exports,
                is_public_python_name(node, source),
            ) {
                return;
            }

            if let Some(signature) = extract_tree_sitter_prefix_signature(node, source, "body") {
                signatures.push(format!("python:{signature}"));
            }
            if let Some(body) = node.child_by_field_name("body") {
                let next_namespace = extend_python_class_namespace(class_namespace, node, source);
                collect_public_api(
                    body,
                    source,
                    signatures,
                    false,
                    &next_namespace,
                    extraction_context,
                );
            }
        }
        "assignment" => {
            if !inside_callable {
                let is_explicit_export = extract_python_assignment_name(node, source)
                    .as_deref()
                    .map(|name| {
                        python_is_explicit_export(
                            name,
                            class_namespace,
                            extraction_context.explicit_exports,
                        )
                    })
                    .unwrap_or(false);
                if let Some(signature) = extract_python_constant_signature(
                    node,
                    source,
                    class_namespace,
                    extraction_context.explicit_exports,
                    is_explicit_export,
                ) {
                    signatures.push(format!("python:{signature}"));
                }
            }
        }
        "type_alias_statement" => {
            if !inside_callable {
                let alias_name = node
                    .child_by_field_name("left")
                    .and_then(|child| node_text(child, source))
                    .map(normalize_tree_sitter_signature)
                    .and_then(|left| python_type_alias_name(&left).map(ToString::to_string));
                let is_explicit_export = alias_name
                    .as_deref()
                    .map(|name| {
                        python_is_explicit_export(
                            name,
                            class_namespace,
                            extraction_context.explicit_exports,
                        )
                    })
                    .unwrap_or(false);
                if let Some(signature) = extract_python_type_alias_signature(
                    node,
                    source,
                    class_namespace,
                    extraction_context.explicit_exports,
                    is_explicit_export,
                ) {
                    signatures.push(format!("python:{signature}"));
                }
            }
        }
        "decorated_definition" => {
            if let Some(definition) = node.child_by_field_name("definition") {
                collect_definition(
                    definition,
                    source,
                    signatures,
                    inside_callable,
                    class_namespace,
                    extraction_context,
                );
            }
        }
        "import_statement" | "import_from_statement" => {
            if inside_callable || !class_namespace.is_empty() {
                return;
            }

            signatures.extend(
                extract_python_import_reexport_signatures(
                    node,
                    source,
                    extraction_context.explicit_exports,
                    extraction_context.import_context,
                    extraction_context.top_level_bindings,
                )
                .into_iter()
                .map(|signature| format!("python:{signature}")),
            );
        }
        "expression_statement" | "module" | "block" => collect_public_api(
            node,
            source,
            signatures,
            inside_callable,
            class_namespace,
            extraction_context,
        ),
        _ => {}
    }
}

fn extract_python_explicit_exports(
    root: Node<'_>,
    source: &str,
    import_context: &PythonImportContext,
) -> Option<BTreeSet<String>> {
    let mut exports = BTreeSet::new();
    let mut bindings = BTreeMap::new();
    let mut module_aliases = BTreeMap::new();
    let mut found_explicit_boundary = false;
    if !collect_python_explicit_exports(
        root,
        source,
        &mut exports,
        &mut bindings,
        &mut module_aliases,
        import_context,
        &mut found_explicit_boundary,
    ) {
        return None;
    }

    found_explicit_boundary.then_some(exports)
}

fn collect_python_explicit_exports(
    node: Node<'_>,
    source: &str,
    exports: &mut BTreeSet<String>,
    bindings: &mut BTreeMap<String, BTreeSet<String>>,
    module_aliases: &mut BTreeMap<String, BTreeSet<String>>,
    import_context: &PythonImportContext,
    found_explicit_boundary: &mut bool,
) -> bool {
    match node.kind() {
        "module" | "expression_statement" => named_children(node).into_iter().all(|child| {
            collect_python_explicit_exports(
                child,
                source,
                exports,
                bindings,
                module_aliases,
                import_context,
                found_explicit_boundary,
            )
        }),
        "assignment" => {
            let Some(name) = extract_python_assignment_name(node, source) else {
                return true;
            };

            let Some(right) = node.child_by_field_name("right") else {
                return name != "__all__";
            };
            let Some(names) =
                extract_python_explicit_export_names(right, source, bindings, module_aliases)
            else {
                bindings.remove(&name);
                return name != "__all__";
            };

            if name == "__all__" {
                *found_explicit_boundary = true;
                exports.clear();
                exports.extend(names.clone());
            };

            bindings.insert(name, names);
            true
        }
        "augmented_assignment" => {
            let left = node.child_by_field_name("left");
            let left_name = left
                .and_then(|child| node_text(child, source))
                .map(normalize_tree_sitter_signature);
            let Some(name) = left_name else {
                return true;
            };

            let Some(right) = node.child_by_field_name("right") else {
                return name != "__all__";
            };
            let Some(names) =
                extract_python_explicit_export_names(right, source, bindings, module_aliases)
            else {
                bindings.remove(&name);
                return name != "__all__";
            };

            let binding = bindings.entry(name.clone()).or_default();
            binding.extend(names.clone());

            if name == "__all__" {
                *found_explicit_boundary = true;
                exports.extend(names);
            };
            true
        }
        "import_statement" | "import_from_statement" => {
            apply_python_import_bindings_from_context(
                node,
                source,
                bindings,
                module_aliases,
                import_context,
            );
            true
        }
        _ => true,
    }
}

fn extract_python_explicit_export_names(
    node: Node<'_>,
    source: &str,
    bindings: &BTreeMap<String, BTreeSet<String>>,
    module_aliases: &BTreeMap<String, BTreeSet<String>>,
) -> Option<BTreeSet<String>> {
    match node.kind() {
        "parenthesized_expression" => named_children(node).into_iter().next().and_then(|child| {
            extract_python_explicit_export_names(child, source, bindings, module_aliases)
        }),
        "list_splat" | "parenthesized_list_splat" => {
            named_children(node).into_iter().next().and_then(|child| {
                extract_python_explicit_export_names(child, source, bindings, module_aliases)
            })
        }
        "list" | "tuple" | "set" | "expression_list" => {
            let mut exports = BTreeSet::new();
            for child in named_children(node) {
                exports.extend(extract_python_explicit_export_names(
                    child,
                    source,
                    bindings,
                    module_aliases,
                )?);
            }
            Some(exports)
        }
        "binary_operator" => {
            let left = node.child_by_field_name("left")?;
            let right = node.child_by_field_name("right")?;
            let mut exports =
                extract_python_explicit_export_names(left, source, bindings, module_aliases)?;
            exports.extend(extract_python_explicit_export_names(
                right,
                source,
                bindings,
                module_aliases,
            )?);
            Some(exports)
        }
        "attribute" => {
            let text = node_text(node, source).map(normalize_tree_sitter_signature)?;
            let (module_alias, attr_name) = text.rsplit_once('.')?;
            (attr_name == "__all__").then(|| module_aliases.get(module_alias).cloned())?
        }
        "identifier" => node_text(node, source)
            .map(normalize_tree_sitter_signature)
            .and_then(|name| bindings.get(&name).cloned()),
        "string" => {
            let name = parse_python_string_literal(node_text(node, source)?)?;
            let mut exports = BTreeSet::new();
            exports.insert(name);
            Some(exports)
        }
        _ => None,
    }
}

fn parse_python_string_literal(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let quote_start = raw.find(|character| character == '\'' || character == '"')?;
    let prefix = &raw[..quote_start];
    if !prefix
        .chars()
        .all(|character| character.is_ascii_alphabetic())
    {
        return None;
    }

    let content = &raw[quote_start..];
    if content.starts_with("'''") || content.starts_with("\"\"\"") {
        return None;
    }

    let quote = content.chars().next()?;
    if !content.ends_with(quote) || content.len() < 2 {
        return None;
    }

    Some(content[1..content.len() - 1].to_string())
}

fn extract_python_import_reexport_signatures(
    node: Node<'_>,
    source: &str,
    explicit_exports: Option<&BTreeSet<String>>,
    import_context: &PythonImportContext,
    top_level_bindings: &BTreeSet<String>,
) -> Vec<String> {
    let Some(explicit_exports) = explicit_exports else {
        return Vec::new();
    };

    let mut signatures: Vec<String> = extract_python_import_bindings(node, source)
        .into_iter()
        .filter(|(binding_name, _)| explicit_exports.contains(binding_name))
        .map(|(_, signature)| signature)
        .collect();

    if node.kind() == "import_from_statement" && node.child_by_field_name("name").is_none() {
        let Some(module_name) = node
            .child_by_field_name("module_name")
            .and_then(|child| node_text(child, source))
            .map(normalize_tree_sitter_signature)
            .filter(|value| !value.is_empty())
        else {
            return signatures;
        };

        let Some(wildcard) = import_context
            .wildcard_imports
            .iter()
            .find(|import| import.module_name == module_name)
        else {
            return signatures;
        };

        for export_name in explicit_exports {
            if top_level_bindings.contains(export_name) || !wildcard.exports.contains(export_name) {
                continue;
            }
            signatures.push(format!("from {module_name} import {export_name}"));
        }
    }

    signatures
}

fn extract_python_import_bindings(node: Node<'_>, source: &str) -> Vec<(String, String)> {
    match node.kind() {
        "import_statement" => children_by_field_name(node, "name")
            .into_iter()
            .filter_map(|child| extract_python_import_binding(child, source))
            .collect(),
        "import_from_statement" => {
            let Some(module_name) = node
                .child_by_field_name("module_name")
                .and_then(|child| node_text(child, source))
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())
            else {
                return Vec::new();
            };

            children_by_field_name(node, "name")
                .into_iter()
                .filter_map(|child| extract_python_from_import_binding(child, source, &module_name))
                .collect()
        }
        _ => Vec::new(),
    }
}

fn extract_python_import_binding(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "aliased_import" => {
            let import_name = node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source))
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())?;
            let alias = node
                .child_by_field_name("alias")
                .and_then(|child| node_text(child, source))
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())?;
            Some((alias.clone(), format!("import {import_name} as {alias}")))
        }
        "dotted_name" => {
            let import_name = node_text(node, source)
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())?;
            let binding_name = import_name
                .split('.')
                .next()
                .map(ToString::to_string)
                .filter(|value| !value.is_empty())?;
            Some((binding_name, format!("import {import_name}")))
        }
        _ => None,
    }
}

fn extract_python_from_import_binding(
    node: Node<'_>,
    source: &str,
    module_name: &str,
) -> Option<(String, String)> {
    match node.kind() {
        "aliased_import" => {
            let import_name = node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source))
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())?;
            let alias = node
                .child_by_field_name("alias")
                .and_then(|child| node_text(child, source))
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())?;
            Some((
                alias.clone(),
                format!("from {module_name} import {import_name} as {alias}"),
            ))
        }
        "dotted_name" => {
            let import_name = node_text(node, source)
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())?;
            let binding_name = import_name
                .rsplit('.')
                .next()
                .map(ToString::to_string)
                .filter(|value| !value.is_empty())?;
            Some((
                binding_name,
                format!("from {module_name} import {import_name}"),
            ))
        }
        _ => None,
    }
}

fn build_python_import_context(
    root: Node<'_>,
    source: &str,
    module_index: Option<&PythonModuleIndex>,
) -> PythonImportContext {
    let Some(module_index) = module_index else {
        return PythonImportContext::default();
    };

    let mut context = PythonImportContext::default();
    for child in named_children(root) {
        collect_python_import_context(child, source, module_index, &mut context);
    }
    context
}

fn collect_python_import_context(
    node: Node<'_>,
    source: &str,
    module_index: &PythonModuleIndex,
    context: &mut PythonImportContext,
) {
    match node.kind() {
        "module" | "expression_statement" => {
            for child in named_children(node) {
                collect_python_import_context(child, source, module_index, context);
            }
        }
        "import_statement" | "import_from_statement" => {
            collect_python_import_context_from_module_index(node, source, context, module_index);
        }
        _ => {}
    }
}

fn apply_python_import_bindings_from_context(
    node: Node<'_>,
    source: &str,
    bindings: &mut BTreeMap<String, BTreeSet<String>>,
    module_aliases: &mut BTreeMap<String, BTreeSet<String>>,
    import_context: &PythonImportContext,
) {
    match node.kind() {
        "import_statement" => {
            for child in children_by_field_name(node, "name") {
                if let Some((alias, _module_name)) =
                    extract_python_module_alias_binding(child, source)
                {
                    if let Some(exports) = import_context.imported_module_exports.get(&alias) {
                        module_aliases.insert(alias, exports.clone());
                    }
                }
            }
        }
        "import_from_statement" => {
            for child in children_by_field_name(node, "name") {
                if let Some((binding_name, imported_name)) =
                    extract_python_from_import_name(child, source)
                {
                    if imported_name == "__all__" {
                        if let Some(exports) =
                            import_context.imported_export_sets.get(&binding_name)
                        {
                            bindings.insert(binding_name, exports.clone());
                        }
                    } else if import_context
                        .imported_export_sets
                        .contains_key(&binding_name)
                    {
                        let mut export_names = BTreeSet::new();
                        export_names.insert(binding_name.clone());
                        bindings.insert(binding_name, export_names);
                    }
                }
            }
        }
        _ => {}
    }
}

fn collect_python_import_context_from_module_index(
    node: Node<'_>,
    source: &str,
    context: &mut PythonImportContext,
    module_index: &PythonModuleIndex,
) {
    match node.kind() {
        "import_statement" => {
            for child in children_by_field_name(node, "name") {
                if let Some((alias, module_name)) =
                    extract_python_module_alias_binding(child, source)
                {
                    if let Some(exports) = module_index.exports_by_module.get(&module_name) {
                        context
                            .imported_module_exports
                            .insert(alias, exports.clone());
                    }
                }
            }
        }
        "import_from_statement" => {
            let Some(module_name) = node
                .child_by_field_name("module_name")
                .and_then(|child| node_text(child, source))
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())
            else {
                return;
            };

            let Some(module_exports) = module_index.exports_by_module.get(&module_name).cloned()
            else {
                return;
            };

            if node.child_by_field_name("name").is_none() {
                context.wildcard_imports.push(PythonWildcardImport {
                    module_name,
                    exports: module_exports,
                });
                return;
            }

            for child in children_by_field_name(node, "name") {
                if let Some((binding_name, imported_name)) =
                    extract_python_from_import_name(child, source)
                {
                    if imported_name == "__all__" {
                        context
                            .imported_export_sets
                            .insert(binding_name, module_exports.clone());
                    } else if module_exports.contains(&imported_name) {
                        let mut export_names = BTreeSet::new();
                        export_names.insert(binding_name.clone());
                        context
                            .imported_export_sets
                            .insert(binding_name, export_names);
                    }
                }
            }
        }
        _ => {}
    }
}

fn extract_python_module_alias_binding(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "aliased_import" => {
            let module_name = node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source))
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())?;
            let alias = node
                .child_by_field_name("alias")
                .and_then(|child| node_text(child, source))
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())?;
            Some((alias, module_name))
        }
        "dotted_name" => {
            let module_name = node_text(node, source)
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())?;
            let binding_name = module_name
                .split('.')
                .next()
                .map(ToString::to_string)
                .filter(|value| !value.is_empty())?;
            Some((binding_name, module_name))
        }
        _ => None,
    }
}

fn extract_python_from_import_name(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "aliased_import" => {
            let import_name = node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source))
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())?;
            let alias = node
                .child_by_field_name("alias")
                .and_then(|child| node_text(child, source))
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())?;
            Some((alias, import_name))
        }
        "dotted_name" => {
            let import_name = node_text(node, source)
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())?;
            let binding_name = import_name
                .rsplit('.')
                .next()
                .map(ToString::to_string)
                .filter(|value| !value.is_empty())?;
            Some((binding_name, import_name))
        }
        _ => None,
    }
}

fn summarize_python_module_exports(
    source: &str,
    module_index: &PythonModuleIndex,
) -> Option<BTreeSet<String>> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(source, None)?;
    let signatures = extract(tree.root_node(), source, "", Some(module_index));
    Some(extract_python_export_names_from_signatures(&signatures))
}

fn python_module_name_candidates(
    rel_path: &str,
    export_following: PythonExportFollowing,
    module_roots: &[String],
) -> Vec<String> {
    let normalized = rel_path.replace('\\', "/");
    let Some(stripped) = normalized.strip_suffix(".py") else {
        return Vec::new();
    };

    let mut candidates = Vec::new();

    if export_following == PythonExportFollowing::Heuristic && module_roots.is_empty() {
        let mut parts: Vec<&str> = stripped
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();
        if parts.last() == Some(&"__init__") {
            parts.pop();
        }
        if parts.is_empty() {
            return Vec::new();
        }

        candidates.push(parts.join("."));
        if matches!(parts.first().copied(), Some("src" | "lib" | "python")) && parts.len() > 1 {
            candidates.push(parts[1..].join("."));
        }
    } else {
        for root in module_roots {
            let normalized_root = root.trim_matches('/').replace('\\', "/");
            let Some(remainder) = stripped
                .strip_prefix(&normalized_root)
                .and_then(|value| value.strip_prefix('/').or(Some(value)))
            else {
                continue;
            };

            let mut parts: Vec<&str> = remainder
                .split('/')
                .filter(|part| !part.is_empty())
                .collect();
            if parts.last() == Some(&"__init__") {
                parts.pop();
            }
            if parts.is_empty() {
                continue;
            }

            candidates.push(parts.join("."));
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn extract_python_export_names_from_signatures(signatures: &[String]) -> BTreeSet<String> {
    let mut exports = BTreeSet::new();

    for signature in signatures {
        if let Some(name) = python_export_name_from_signature(signature) {
            exports.insert(name);
        }
    }

    exports
}

fn python_export_name_from_signature(signature: &str) -> Option<String> {
    if let Some(rest) = signature.strip_prefix("python:class ") {
        return rest
            .split(['(', ':', ' '])
            .next()
            .map(ToString::to_string)
            .filter(|value| !value.is_empty());
    }

    if let Some(rest) = signature.strip_prefix("python:def ") {
        let name = rest.split('(').next()?.trim();
        return (!name.contains('.'))
            .then(|| name.to_string())
            .filter(|value| !value.is_empty());
    }

    if let Some(rest) = signature.strip_prefix("python:type ") {
        let name = rest.split('=').next()?.trim();
        return (!name.contains('.'))
            .then(|| name.to_string())
            .filter(|value| !value.is_empty());
    }

    if let Some(rest) = signature.strip_prefix("python:const ") {
        let name = rest.split(':').next()?.trim();
        return (!name.contains('.') && name != "__all__")
            .then(|| name.to_string())
            .filter(|value| !value.is_empty());
    }

    if let Some(rest) = signature.strip_prefix("python:from ") {
        let import_name = rest.split(" import ").nth(1)?;
        let binding = import_name
            .split(" as ")
            .last()
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        return Some(binding.to_string());
    }

    if let Some(rest) = signature.strip_prefix("python:import ") {
        let binding = rest
            .split(" as ")
            .nth(1)
            .or_else(|| rest.split('.').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        return Some(binding.to_string());
    }

    None
}

fn collect_python_top_level_bindings(root: Node<'_>, source: &str) -> BTreeSet<String> {
    let mut bindings = BTreeSet::new();
    collect_python_top_level_bindings_into(root, source, &mut bindings);
    bindings
}

fn collect_python_top_level_bindings_into(
    node: Node<'_>,
    source: &str,
    bindings: &mut BTreeSet<String>,
) {
    match node.kind() {
        "module" | "expression_statement" => {
            for child in named_children(node) {
                collect_python_top_level_bindings_into(child, source, bindings);
            }
        }
        "function_definition" | "class_definition" => {
            if let Some(name) = python_definition_name(node, source) {
                bindings.insert(name);
            }
        }
        "assignment" => {
            if let Some(name) = extract_python_assignment_name(node, source) {
                bindings.insert(name);
            }
        }
        "type_alias_statement" => {
            let alias_name = node
                .child_by_field_name("left")
                .and_then(|child| node_text(child, source))
                .map(normalize_tree_sitter_signature)
                .and_then(|left| python_type_alias_name(&left).map(ToString::to_string));
            if let Some(name) = alias_name {
                bindings.insert(name);
            }
        }
        "decorated_definition" => {
            if let Some(definition) = node.child_by_field_name("definition") {
                collect_python_top_level_bindings_into(definition, source, bindings);
            }
        }
        "import_statement" => {
            for child in children_by_field_name(node, "name") {
                if let Some((binding_name, _)) = extract_python_import_binding(child, source) {
                    bindings.insert(binding_name);
                }
            }
        }
        "import_from_statement" => {
            for child in children_by_field_name(node, "name") {
                if let Some((binding_name, _)) =
                    extract_python_from_import_binding(child, source, "")
                {
                    bindings.insert(binding_name);
                }
            }
        }
        _ => {}
    }
}

fn extend_python_class_namespace(
    namespace: &[String],
    node: Node<'_>,
    source: &str,
) -> Vec<String> {
    let mut next = namespace.to_vec();
    let Some(name) = node
        .child_by_field_name("name")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
    else {
        return next;
    };
    next.push(name);
    next
}

fn extract_python_constant_signature(
    node: Node<'_>,
    source: &str,
    class_namespace: &[String],
    explicit_exports: Option<&BTreeSet<String>>,
    is_explicit_export: bool,
) -> Option<String> {
    let name = extract_python_assignment_name(node, source)?;
    if name == "__all__" && !class_namespace.is_empty() {
        return None;
    }
    if !python_should_include_name(
        &name,
        class_namespace,
        explicit_exports,
        is_explicit_export || is_public_python_constant_name(&name),
    ) {
        return None;
    }
    let qualified_name = qualify_python_member_name(class_namespace, &name);

    let type_annotation = node
        .child_by_field_name("type")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty());

    Some(match type_annotation {
        Some(type_annotation) => format!("const {qualified_name}: {type_annotation}"),
        None => format!("const {qualified_name}"),
    })
}

fn extract_python_type_alias_signature(
    node: Node<'_>,
    source: &str,
    class_namespace: &[String],
    explicit_exports: Option<&BTreeSet<String>>,
    is_explicit_export: bool,
) -> Option<String> {
    let left = node
        .child_by_field_name("left")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())?;
    let alias_name = python_type_alias_name(&left)?;
    if !python_should_include_name(
        alias_name,
        class_namespace,
        explicit_exports,
        is_explicit_export || is_public_python_identifier(alias_name),
    ) {
        return None;
    }

    let right = node
        .child_by_field_name("right")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())?;
    let qualified_left = qualify_python_member_name(class_namespace, &left);

    Some(format!("type {qualified_left} = {right}"))
}

fn extract_python_assignment_name(node: Node<'_>, source: &str) -> Option<String> {
    let left = node.child_by_field_name("left")?;
    let mut names = Vec::new();
    collect_python_assignment_identifiers(left, source, &mut names);
    if names.len() == 1 {
        names.into_iter().next()
    } else {
        None
    }
}

fn collect_python_assignment_identifiers(node: Node<'_>, source: &str, names: &mut Vec<String>) {
    if node.kind() == "identifier" {
        if let Some(name) = node_text(node, source).map(normalize_tree_sitter_signature) {
            names.push(name);
        }
        return;
    }

    for child in named_children(node) {
        collect_python_assignment_identifiers(child, source, names);
    }
}

fn qualify_python_member_name(namespace: &[String], name: &str) -> String {
    if namespace.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", namespace.join("."), name)
    }
}

fn qualify_python_function_signature(signature: &str, class_namespace: &[String]) -> String {
    if class_namespace.is_empty() {
        return signature.to_string();
    }

    let Some(rest) = signature.strip_prefix("def ") else {
        return signature.to_string();
    };

    format!("def {}.{rest}", class_namespace.join("."))
}

fn python_type_alias_name(signature: &str) -> Option<&str> {
    let alias = signature.split('[').next().unwrap_or(signature).trim();
    (!alias.is_empty()).then_some(alias)
}

fn python_definition_name(node: Node<'_>, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
}

fn python_should_include_name(
    name: &str,
    class_namespace: &[String],
    explicit_exports: Option<&BTreeSet<String>>,
    default_public: bool,
) -> bool {
    if !class_namespace.is_empty() {
        return default_public;
    }

    explicit_exports
        .map(|exports| name == "__all__" || exports.contains(name))
        .unwrap_or(default_public)
}

fn python_is_explicit_export(
    name: &str,
    class_namespace: &[String],
    explicit_exports: Option<&BTreeSet<String>>,
) -> bool {
    class_namespace.is_empty()
        && explicit_exports
            .map(|exports| exports.contains(name))
            .unwrap_or(false)
}

fn is_public_python_identifier(name: &str) -> bool {
    !name.starts_with('_')
}

fn is_public_python_constant_name(name: &str) -> bool {
    name == "__all__"
        || (!name.starts_with('_')
            && name
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_'))
}
