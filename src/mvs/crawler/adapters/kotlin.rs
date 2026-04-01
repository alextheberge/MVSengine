// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::super::{
    extract_tree_sitter_prefix_before_named_child, named_children, normalize_kotlin_signature,
};

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_public_api(root, source, &mut signatures);
    signatures
}

fn collect_public_api(node: Node<'_>, source: &str, signatures: &mut Vec<String>) {
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

                for nested in named_children(child) {
                    if matches!(nested.kind(), "class_body" | "enum_class_body") {
                        collect_public_api(nested, source, signatures);
                    }
                }
            }
            "function_declaration" => {
                if let Some(signature) =
                    extract_tree_sitter_prefix_before_named_child(child, source, &["function_body"])
                        .and_then(|value| normalize_kotlin_signature(&value))
                {
                    signatures.push(format!("kotlin:{signature}"));
                }
            }
            "class_body" | "enum_class_body" => collect_public_api(child, source, signatures),
            _ => {}
        }
    }
}
