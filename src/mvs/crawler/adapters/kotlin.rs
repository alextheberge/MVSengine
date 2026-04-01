// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::super::{
    extract_tree_sitter_prefix_before_named_child, named_children, node_text,
    normalize_kotlin_signature, normalize_tree_sitter_signature,
};

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_public_api(root, source, &mut signatures, &[]);
    signatures
}

fn collect_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    type_namespace: &[String],
) {
    for child in named_children(node) {
        match child.kind() {
            "class_declaration" | "object_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_before_named_child(
                    child,
                    source,
                    &["class_body", "enum_class_body"],
                )
                .and_then(|value| normalize_kotlin_signature(&value))
                {
                    signatures.push(format!("kotlin:{signature}"));
                }

                let next_namespace = extend_kotlin_type_namespace(type_namespace, child, source);
                for nested in named_children(child) {
                    if matches!(nested.kind(), "class_body" | "enum_class_body") {
                        collect_public_api(nested, source, signatures, &next_namespace);
                    }
                }
            }
            "function_declaration" => {
                if let Some(signature) =
                    extract_tree_sitter_prefix_before_named_child(child, source, &["function_body"])
                        .and_then(|value| normalize_kotlin_signature(&value))
                {
                    signatures.push(format!(
                        "kotlin:{}",
                        qualify_kotlin_function_signature(
                            &signature,
                            child,
                            source,
                            type_namespace
                        )
                    ));
                }
            }
            "class_body" | "enum_class_body" => {
                collect_public_api(child, source, signatures, type_namespace)
            }
            _ => {}
        }
    }
}

fn extend_kotlin_type_namespace(namespace: &[String], node: Node<'_>, source: &str) -> Vec<String> {
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

fn qualify_kotlin_function_signature(
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
