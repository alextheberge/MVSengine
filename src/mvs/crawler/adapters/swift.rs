// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::super::{
    extract_tree_sitter_prefix_before_fields, extract_tree_sitter_prefix_before_named_child,
    extract_tree_sitter_prefix_signature, named_children, node_text,
    normalize_swift_protocol_member_signature, normalize_swift_signature,
    normalize_tree_sitter_signature, swift_visibility_keyword,
};

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_public_api(root, source, &mut signatures, None);
    signatures
}

fn collect_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    exported_protocol_visibility: Option<&str>,
) {
    for child in named_children(node) {
        match child.kind() {
            "class_declaration"
            | "struct_declaration"
            | "enum_declaration"
            | "extension_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_swift_signature(&value))
                {
                    signatures.push(format!("swift:{signature}"));
                }

                if let Some(body) = child.child_by_field_name("body") {
                    collect_public_api(body, source, signatures, None);
                }
            }
            "protocol_declaration" => {
                let normalized = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_swift_signature(&value));
                let visibility = normalized.as_deref().and_then(swift_visibility_keyword);

                if let Some(signature) = normalized.as_deref() {
                    signatures.push(format!("swift:{signature}"));
                }

                if let Some(body) = child.child_by_field_name("body") {
                    collect_public_api(body, source, signatures, visibility);
                }
            }
            "function_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_swift_signature(&value))
                {
                    signatures.push(format!("swift:{signature}"));
                }
            }
            "property_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_before_fields(
                    child,
                    source,
                    &["computed_value", "value"],
                )
                .and_then(|value| normalize_swift_signature(&value))
                {
                    signatures.push(format!("swift:{signature}"));
                }
            }
            "protocol_function_declaration" => {
                if let Some(signature) =
                    extract_tree_sitter_prefix_before_named_child(child, source, &["statements"])
                        .and_then(|value| {
                            normalize_swift_protocol_member_signature(
                                &value,
                                exported_protocol_visibility,
                            )
                        })
                {
                    signatures.push(format!("swift:{signature}"));
                }
            }
            "protocol_property_declaration" => {
                if let Some(signature) = node_text(child, source)
                    .map(normalize_tree_sitter_signature)
                    .and_then(|value| {
                        normalize_swift_protocol_member_signature(
                            &value,
                            exported_protocol_visibility,
                        )
                    })
                {
                    signatures.push(format!("swift:{signature}"));
                }
            }
            "source_file" | "class_body" | "protocol_body" | "enum_class_body" => {
                collect_public_api(child, source, signatures, exported_protocol_visibility);
            }
            _ => {}
        }
    }
}
