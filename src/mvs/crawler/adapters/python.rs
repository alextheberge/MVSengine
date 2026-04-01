// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::super::{
    extract_tree_sitter_prefix_signature, is_public_python_name, named_children, node_text,
    normalize_tree_sitter_signature,
};

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_public_api(root, source, &mut signatures, false, &[]);
    signatures
}

fn collect_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    inside_callable: bool,
    class_namespace: &[String],
) {
    for child in named_children(node) {
        collect_definition(child, source, signatures, inside_callable, class_namespace);
    }
}

fn collect_definition(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    inside_callable: bool,
    class_namespace: &[String],
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
            if !inside_callable && is_public_python_name(node, source) {
                if let Some(signature) = extract_tree_sitter_prefix_signature(node, source, "body")
                {
                    signatures.push(format!("python:{signature}"));
                }
                if let Some(body) = node.child_by_field_name("body") {
                    let next_namespace =
                        extend_python_class_namespace(class_namespace, node, source);
                    collect_public_api(body, source, signatures, false, &next_namespace);
                }
            }
        }
        "assignment" => {
            if !inside_callable {
                if let Some(signature) =
                    extract_python_constant_signature(node, source, class_namespace)
                {
                    signatures.push(format!("python:{signature}"));
                }
            }
        }
        "type_alias_statement" => {
            if !inside_callable {
                if let Some(signature) =
                    extract_python_type_alias_signature(node, source, class_namespace)
                {
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
                );
            }
        }
        "expression_statement" | "module" | "block" => {
            collect_public_api(node, source, signatures, inside_callable, class_namespace)
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
) -> Option<String> {
    let name = extract_python_assignment_name(node, source)?;
    if name == "__all__" && !class_namespace.is_empty() {
        return None;
    }
    if !is_public_python_constant_name(&name) {
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
) -> Option<String> {
    let left = node
        .child_by_field_name("left")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())?;
    let alias_name = python_type_alias_name(&left)?;
    if !is_public_python_identifier(alias_name) {
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

fn python_type_alias_name(signature: &str) -> Option<&str> {
    let alias = signature.split('[').next().unwrap_or(signature).trim();
    (!alias.is_empty()).then_some(alias)
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
