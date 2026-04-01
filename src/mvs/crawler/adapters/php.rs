// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::super::{
    extract_tree_sitter_php_const_signatures, extract_tree_sitter_php_property_signatures,
    extract_tree_sitter_prefix_signature, named_children, node_text,
    normalize_php_function_signature, normalize_php_method_signature, normalize_php_type_signature,
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
    inside_interface: bool,
    type_namespace: &[String],
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
                    let next_namespace = extend_php_type_namespace(type_namespace, child, source);
                    collect_public_api(body, source, signatures, false, &next_namespace);
                }
            }
            "interface_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_php_type_signature(&value))
                {
                    signatures.push(format!("php:{signature}"));
                }

                if let Some(body) = child.child_by_field_name("body") {
                    let next_namespace = extend_php_type_namespace(type_namespace, child, source);
                    collect_public_api(body, source, signatures, true, &next_namespace);
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
                    signatures.push(format!(
                        "php:{}",
                        qualify_php_method_signature(&signature, child, source, type_namespace)
                    ));
                }
            }
            "property_declaration" => {
                signatures.extend(
                    extract_tree_sitter_php_property_signatures(child, source, inside_interface)
                        .into_iter()
                        .map(|signature| {
                            let raw = signature.strip_prefix("php:").unwrap_or(&signature);
                            format!(
                                "php:{}",
                                qualify_php_property_signature(raw, type_namespace)
                            )
                        }),
                );
            }
            "const_declaration" => {
                signatures.extend(
                    extract_tree_sitter_php_const_signatures(child, source, inside_interface)
                        .into_iter()
                        .map(|signature| {
                            let raw = signature.strip_prefix("php:").unwrap_or(&signature);
                            format!("php:{}", qualify_php_const_signature(raw, type_namespace))
                        }),
                );
            }
            "program" | "compound_statement" | "declaration_list" => {
                collect_public_api(child, source, signatures, inside_interface, type_namespace);
            }
            "namespace_definition" => {
                if let Some(body) = child.child_by_field_name("body") {
                    collect_public_api(body, source, signatures, inside_interface, type_namespace);
                }
            }
            _ => {}
        }
    }
}

fn extend_php_type_namespace(namespace: &[String], node: Node<'_>, source: &str) -> Vec<String> {
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

fn qualify_php_method_signature(
    signature: &str,
    node: Node<'_>,
    source: &str,
    type_namespace: &[String],
) -> String {
    let owner = type_namespace.join(".");
    if owner.is_empty() {
        return signature.to_string();
    }

    let Some(name) = node
        .child_by_field_name("name")
        .and_then(|child| node_text(child, source))
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return signature.to_string();
    };

    let needle = format!(" {name}(");
    let replacement = format!(" {owner}.{name}(");
    signature.replacen(&needle, &replacement, 1)
}

fn qualify_php_property_signature(signature: &str, type_namespace: &[String]) -> String {
    let owner = type_namespace.join(".");
    if owner.is_empty() {
        return signature.to_string();
    }

    let Some(index) = signature.rfind(" $") else {
        return signature.to_string();
    };

    format!(
        "{} {}.{}",
        &signature[..index],
        owner,
        &signature[index + 1..]
    )
}

fn qualify_php_const_signature(signature: &str, type_namespace: &[String]) -> String {
    let owner = type_namespace.join(".");
    if owner.is_empty() {
        return signature.to_string();
    }

    let Some(index) = signature.rfind(' ') else {
        return signature.to_string();
    };

    format!(
        "{} {}::{}",
        &signature[..index],
        owner,
        &signature[index + 1..]
    )
}
