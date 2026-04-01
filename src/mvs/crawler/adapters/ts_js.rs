// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::super::{
    extract_tree_sitter_prefix_signature, node_text, node_text_range,
    normalize_export_statement_signature, normalize_signature,
};

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    for child in super::super::named_children(root) {
        if child.kind() != "export_statement" {
            continue;
        }

        signatures.extend(extract_export_statement(child, source));
    }

    signatures
}

fn extract_export_statement(node: Node<'_>, source: &str) -> Vec<String> {
    let is_default_export = node_text(node, source)
        .map(|text| text.trim_start().starts_with("export default"))
        .unwrap_or(false);

    if let Some(declaration) = node.child_by_field_name("declaration") {
        let signatures = extract_export_declaration(declaration, source);
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
        if let Some(signature) = extract_default_export_value(value, source) {
            return vec![format!("ts/js:export default {signature}")];
        }
    }

    node_text(node, source)
        .map(normalize_export_statement_signature)
        .filter(|signature| !signature.is_empty())
        .map(|signature| vec![format!("ts/js:{signature}")])
        .unwrap_or_default()
}

fn extract_export_declaration(node: Node<'_>, source: &str) -> Vec<String> {
    match node.kind() {
        "function_declaration" | "generator_function_declaration" => {
            extract_function_signature(node, source)
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
        "lexical_declaration" => extract_variable_signatures(node, source),
        "variable_declaration" => extract_variable_signatures(node, source),
        _ => Vec::new(),
    }
}

fn extract_default_export_value(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "function" | "function_expression" | "generator_function" => {
            extract_function_signature(node, source)
        }
        "class" => extract_tree_sitter_prefix_signature(node, source, "body"),
        _ => None,
    }
}

fn extract_function_signature(node: Node<'_>, source: &str) -> Option<String> {
    let body = node.child_by_field_name("body")?;
    node_text_range(source, node.start_byte(), body.start_byte())
        .map(normalize_export_statement_signature)
        .filter(|signature| !signature.is_empty())
}

fn extract_variable_signatures(node: Node<'_>, source: &str) -> Vec<String> {
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
