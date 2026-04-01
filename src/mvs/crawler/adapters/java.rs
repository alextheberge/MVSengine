// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::super::{
    extract_tree_sitter_prefix_signature, has_tree_sitter_keyword, named_children, node_text,
    normalize_tree_sitter_signature, trim_signature_to_keywords,
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
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "annotation_type_declaration" => {
                if has_tree_sitter_keyword(child, source, "body", "public") {
                    if let Some(signature) =
                        extract_tree_sitter_prefix_signature(child, source, "body")
                            .and_then(|value| trim_signature_to_keywords(&value, &["public"]))
                    {
                        signatures.push(format!("java:type {signature}"));
                    }
                }

                if let Some(body) = child.child_by_field_name("body") {
                    let next_namespace = extend_java_type_namespace(type_namespace, child, source);
                    collect_public_api(body, source, signatures, &next_namespace);
                }
            }
            "method_declaration" => {
                if has_tree_sitter_keyword(child, source, "body", "public") {
                    if let Some(signature) =
                        extract_tree_sitter_prefix_signature(child, source, "body")
                            .and_then(|value| trim_signature_to_keywords(&value, &["public"]))
                    {
                        signatures.push(format!(
                            "java:method {}",
                            qualify_java_method_signature(
                                &signature,
                                child,
                                source,
                                type_namespace
                            )
                        ));
                    }
                }
            }
            "class_body" | "interface_body" | "annotation_type_body" | "enum_body_declarations" => {
                collect_public_api(child, source, signatures, type_namespace);
            }
            _ => {}
        }
    }
}

fn extend_java_type_namespace(namespace: &[String], node: Node<'_>, source: &str) -> Vec<String> {
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

fn qualify_java_method_signature(
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

    qualify_named_callable_signature(signature, name, &owner)
}

fn qualify_named_callable_signature(signature: &str, name: &str, owner: &str) -> String {
    let needle = format!(" {name}(");
    let replacement = format!(" {owner}.{name}(");
    signature.replacen(&needle, &replacement, 1)
}
