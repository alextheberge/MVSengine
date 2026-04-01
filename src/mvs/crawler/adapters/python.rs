// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::BTreeSet;

use tree_sitter::Node;

use super::super::{
    extract_tree_sitter_prefix_signature, is_public_python_name, named_children, node_text,
    normalize_tree_sitter_signature,
};

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    let explicit_exports = extract_python_explicit_exports(root, source);
    collect_public_api(
        root,
        source,
        &mut signatures,
        false,
        &[],
        explicit_exports.as_ref(),
    );
    signatures
}

fn collect_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    inside_callable: bool,
    class_namespace: &[String],
    explicit_exports: Option<&BTreeSet<String>>,
) {
    for child in named_children(node) {
        collect_definition(
            child,
            source,
            signatures,
            inside_callable,
            class_namespace,
            explicit_exports,
        );
    }
}

fn collect_definition(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    inside_callable: bool,
    class_namespace: &[String],
    explicit_exports: Option<&BTreeSet<String>>,
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
                explicit_exports,
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
                explicit_exports,
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
                    explicit_exports,
                );
            }
        }
        "assignment" => {
            if !inside_callable {
                let is_explicit_export = extract_python_assignment_name(node, source)
                    .as_deref()
                    .map(|name| python_is_explicit_export(name, class_namespace, explicit_exports))
                    .unwrap_or(false);
                if let Some(signature) = extract_python_constant_signature(
                    node,
                    source,
                    class_namespace,
                    explicit_exports,
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
                    .map(|name| python_is_explicit_export(name, class_namespace, explicit_exports))
                    .unwrap_or(false);
                if let Some(signature) = extract_python_type_alias_signature(
                    node,
                    source,
                    class_namespace,
                    explicit_exports,
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
                    explicit_exports,
                );
            }
        }
        "expression_statement" | "module" | "block" => collect_public_api(
            node,
            source,
            signatures,
            inside_callable,
            class_namespace,
            explicit_exports,
        ),
        _ => {}
    }
}

fn extract_python_explicit_exports(root: Node<'_>, source: &str) -> Option<BTreeSet<String>> {
    let mut exports = BTreeSet::new();
    let mut found_explicit_boundary = false;
    if !collect_python_explicit_exports(root, source, &mut exports, &mut found_explicit_boundary) {
        return None;
    }

    found_explicit_boundary.then_some(exports)
}

fn collect_python_explicit_exports(
    node: Node<'_>,
    source: &str,
    exports: &mut BTreeSet<String>,
    found_explicit_boundary: &mut bool,
) -> bool {
    match node.kind() {
        "module" | "expression_statement" => named_children(node).into_iter().all(|child| {
            collect_python_explicit_exports(child, source, exports, found_explicit_boundary)
        }),
        "assignment" => {
            if extract_python_assignment_name(node, source).as_deref() != Some("__all__") {
                return true;
            }

            let Some(right) = node.child_by_field_name("right") else {
                return false;
            };
            let Some(names) = extract_python_explicit_export_names(right, source) else {
                return false;
            };

            *found_explicit_boundary = true;
            exports.extend(names);
            true
        }
        "augmented_assignment" => {
            let left = node.child_by_field_name("left");
            let left_name = left
                .and_then(|child| node_text(child, source))
                .map(normalize_tree_sitter_signature);
            if left_name.as_deref() != Some("__all__") {
                return true;
            }

            let Some(right) = node.child_by_field_name("right") else {
                return false;
            };
            let Some(names) = extract_python_explicit_export_names(right, source) else {
                return false;
            };

            *found_explicit_boundary = true;
            exports.extend(names);
            true
        }
        _ => true,
    }
}

fn extract_python_explicit_export_names(node: Node<'_>, source: &str) -> Option<BTreeSet<String>> {
    match node.kind() {
        "parenthesized_expression" => named_children(node)
            .into_iter()
            .next()
            .and_then(|child| extract_python_explicit_export_names(child, source)),
        "list" | "tuple" | "set" | "expression_list" => {
            let mut exports = BTreeSet::new();
            for child in named_children(node) {
                exports.extend(extract_python_explicit_export_names(child, source)?);
            }
            Some(exports)
        }
        "binary_operator" => {
            let left = node.child_by_field_name("left")?;
            let right = node.child_by_field_name("right")?;
            let mut exports = extract_python_explicit_export_names(left, source)?;
            exports.extend(extract_python_explicit_export_names(right, source)?);
            Some(exports)
        }
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
