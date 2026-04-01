// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::BTreeSet;

use tree_sitter::Node;

use super::super::{
    extract_tree_sitter_prefix_signature, named_children, node_text, normalize_signature,
    normalize_tree_sitter_signature,
};

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_global_public_api(root, source, &mut signatures);
    collect_module_exports(root, source, &mut signatures);
    signatures
}

fn collect_global_public_api(node: Node<'_>, source: &str, signatures: &mut Vec<String>) {
    for child in named_children(node) {
        match child.kind() {
            "chunk" | "statement" => collect_global_public_api(child, source, signatures),
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

fn collect_module_exports(root: Node<'_>, source: &str, signatures: &mut Vec<String>) {
    let module_names = extract_returned_module_names(root, source);
    if module_names.is_empty() {
        return;
    }

    for item in top_level_items(root) {
        match item.kind() {
            "variable_declaration" => {
                let Some(assignment) = named_children(item)
                    .into_iter()
                    .find(|child| child.kind() == "assignment_statement")
                else {
                    continue;
                };
                collect_assignment_exports(assignment, source, &module_names, signatures);
            }
            "assignment_statement" => {
                collect_assignment_exports(item, source, &module_names, signatures);
            }
            "function_declaration" => {
                let Some(name) = item.child_by_field_name("name") else {
                    continue;
                };
                if extract_module_member_name(name, source, &module_names).is_none() {
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

fn collect_assignment_exports(
    assignment: Node<'_>,
    source: &str,
    module_names: &BTreeSet<String>,
    signatures: &mut Vec<String>,
) {
    for (target, value) in extract_assignment_pairs(assignment) {
        if let Some(module_name) = extract_identifier_name(target, source) {
            if module_names.contains(module_name) && value.kind() == "table_constructor" {
                collect_table_constructor_exports(module_name, value, source, signatures);
            }
            continue;
        }

        let Some(member_name) = extract_module_member_name(target, source, module_names) else {
            continue;
        };

        if value.kind() == "function_definition" {
            if let Some(signature) =
                extract_assigned_function_signature(&member_name, value, source)
            {
                signatures.push(format!("luau:{signature}"));
            }
            continue;
        }

        signatures.push(format!("luau:field {member_name}"));
    }
}

fn collect_table_constructor_exports(
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
            if let Some(signature) = extract_assigned_function_signature(&target, value, source) {
                signatures.push(format!("luau:{signature}"));
            }
            continue;
        }

        signatures.push(format!("luau:field {target}"));
    }
}

fn extract_returned_module_names(root: Node<'_>, source: &str) -> BTreeSet<String> {
    let mut module_names = BTreeSet::new();

    for item in top_level_items(root) {
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
            if let Some(name) = extract_identifier_name(expression, source) {
                module_names.insert(name.to_string());
            }
        }
    }

    module_names
}

fn top_level_items(node: Node<'_>) -> Vec<Node<'_>> {
    let mut items = Vec::new();

    for child in named_children(node) {
        match child.kind() {
            "chunk" | "statement" => items.extend(top_level_items(child)),
            _ => items.push(child),
        }
    }

    items
}

fn extract_assignment_pairs(assignment: Node<'_>) -> Vec<(Node<'_>, Node<'_>)> {
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

fn extract_identifier_name<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    let node = unwrap_variable(node);
    if node.kind() != "identifier" {
        return None;
    }

    node_text(node, source)
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

fn extract_module_member_name(
    node: Node<'_>,
    source: &str,
    module_names: &BTreeSet<String>,
) -> Option<String> {
    let node = unwrap_variable(node);
    let separator = match node.kind() {
        "dot_index_expression" => ".",
        "method_index_expression" => ":",
        _ => return None,
    };

    let table = node.child_by_field_name("table")?;
    let table_name = extract_identifier_name(table, source)?;
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

fn unwrap_variable(node: Node<'_>) -> Node<'_> {
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

fn extract_assigned_function_signature(
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
