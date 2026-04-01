// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::super::{
    extract_tree_sitter_php_const_signatures, extract_tree_sitter_php_property_signatures,
    extract_tree_sitter_prefix_signature, named_children, normalize_php_function_signature,
    normalize_php_method_signature, normalize_php_type_signature,
};

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_public_api(root, source, &mut signatures, false);
    signatures
}

fn collect_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    inside_interface: bool,
) {
    for child in named_children(node) {
        match child.kind() {
            "class_declaration" | "trait_declaration" | "enum_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_php_type_signature(&value))
                {
                    signatures.push(format!("php:{signature}"));
                }

                if let Some(body) = child.child_by_field_name("body") {
                    collect_public_api(body, source, signatures, false);
                }
            }
            "interface_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_php_type_signature(&value))
                {
                    signatures.push(format!("php:{signature}"));
                }

                if let Some(body) = child.child_by_field_name("body") {
                    collect_public_api(body, source, signatures, true);
                }
            }
            "function_definition" => {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_php_function_signature(&value))
                {
                    signatures.push(format!("php:{signature}"));
                }
            }
            "method_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_php_method_signature(&value, inside_interface))
                {
                    signatures.push(format!("php:{signature}"));
                }
            }
            "property_declaration" => {
                signatures.extend(extract_tree_sitter_php_property_signatures(
                    child,
                    source,
                    inside_interface,
                ));
            }
            "const_declaration" => {
                signatures.extend(extract_tree_sitter_php_const_signatures(
                    child,
                    source,
                    inside_interface,
                ));
            }
            "program" | "compound_statement" | "declaration_list" => {
                collect_public_api(child, source, signatures, inside_interface);
            }
            "namespace_definition" => {
                if let Some(body) = child.child_by_field_name("body") {
                    collect_public_api(body, source, signatures, inside_interface);
                }
            }
            _ => {}
        }
    }
}
