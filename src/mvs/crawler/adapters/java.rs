// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::super::{
    extract_tree_sitter_prefix_signature, has_tree_sitter_keyword, named_children,
    trim_signature_to_keywords,
};

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_public_api(root, source, &mut signatures);
    signatures
}

fn collect_public_api(node: Node<'_>, source: &str, signatures: &mut Vec<String>) {
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
                    collect_public_api(body, source, signatures);
                }
            }
            "method_declaration" => {
                if has_tree_sitter_keyword(child, source, "body", "public") {
                    if let Some(signature) =
                        extract_tree_sitter_prefix_signature(child, source, "body")
                            .and_then(|value| trim_signature_to_keywords(&value, &["public"]))
                    {
                        signatures.push(format!("java:method {signature}"));
                    }
                }
            }
            "class_body" | "interface_body" | "annotation_type_body" | "enum_body_declarations" => {
                collect_public_api(child, source, signatures);
            }
            _ => {}
        }
    }
}
