// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::super::{extract_tree_sitter_prefix_signature, is_public_python_name, named_children};

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_public_api(root, source, &mut signatures, false);
    signatures
}

fn collect_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    inside_callable: bool,
) {
    for child in named_children(node) {
        collect_definition(child, source, signatures, inside_callable);
    }
}

fn collect_definition(
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
                    collect_public_api(body, source, signatures, false);
                }
            }
        }
        "decorated_definition" => {
            if let Some(definition) = node.child_by_field_name("definition") {
                collect_definition(definition, source, signatures, inside_callable);
            }
        }
        "module" | "block" => collect_public_api(node, source, signatures, inside_callable),
        _ => {}
    }
}
