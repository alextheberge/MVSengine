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
    collect_public_api(root, source, &mut signatures, None, &[]);
    signatures
}

fn collect_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    exported_protocol_visibility: Option<&str>,
    owner_namespace: &[String],
) {
    for child in named_children(node) {
        match child.kind() {
            "class_declaration"
            | "struct_declaration"
            | "enum_declaration"
            | "extension_declaration"
            | "extension" => {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_swift_signature(&value))
                {
                    signatures.push(format!("swift:{signature}"));
                }

                if let Some(body) = child.child_by_field_name("body") {
                    let next_namespace =
                        extend_swift_owner_namespace(owner_namespace, child, source);
                    collect_public_api(body, source, signatures, None, &next_namespace);
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
                    let next_namespace =
                        extend_swift_owner_namespace(owner_namespace, child, source);
                    collect_public_api(body, source, signatures, visibility, &next_namespace);
                }
            }
            "function_declaration" => {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                    .and_then(|value| normalize_swift_signature(&value))
                {
                    signatures.push(format!(
                        "swift:{}",
                        qualify_swift_callable_signature(
                            &signature,
                            child,
                            source,
                            owner_namespace
                        )
                    ));
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
                    signatures.push(format!(
                        "swift:{}",
                        qualify_swift_property_signature(
                            &signature,
                            child,
                            source,
                            owner_namespace
                        )
                    ));
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
                    signatures.push(format!(
                        "swift:{}",
                        qualify_swift_callable_signature(
                            &signature,
                            child,
                            source,
                            owner_namespace
                        )
                    ));
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
                    signatures.push(format!(
                        "swift:{}",
                        qualify_swift_property_signature(
                            &signature,
                            child,
                            source,
                            owner_namespace
                        )
                    ));
                }
            }
            "source_file" | "class_body" | "protocol_body" | "enum_class_body" => {
                collect_public_api(
                    child,
                    source,
                    signatures,
                    exported_protocol_visibility,
                    owner_namespace,
                );
            }
            _ => {}
        }
    }
}

fn extend_swift_owner_namespace(namespace: &[String], node: Node<'_>, source: &str) -> Vec<String> {
    let mut next = namespace.to_vec();
    let direct_name = node
        .child_by_field_name("name")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty());
    let inferred_name =
        if direct_name.is_none() && matches!(node.kind(), "extension" | "extension_declaration") {
            extract_tree_sitter_prefix_signature(node, source, "body")
                .map(|signature| normalize_tree_sitter_signature(&signature))
                .and_then(|signature| parse_swift_extension_owner(&signature))
        } else {
            None
        };
    let Some(name) = direct_name.or(inferred_name) else {
        return next;
    };
    next.push(name);
    next
}

fn qualify_swift_callable_signature(
    signature: &str,
    node: Node<'_>,
    source: &str,
    owner_namespace: &[String],
) -> String {
    let owner = owner_namespace.join(".");
    if owner.is_empty() {
        return signature.to_string();
    }

    let Some(name) = swift_member_name(node, source) else {
        return signature.to_string();
    };

    let needle = format!(" {name}(");
    let replacement = format!(" {owner}.{name}(");
    signature.replacen(&needle, &replacement, 1)
}

fn qualify_swift_property_signature(
    signature: &str,
    node: Node<'_>,
    source: &str,
    owner_namespace: &[String],
) -> String {
    let owner = owner_namespace.join(".");
    if owner.is_empty() {
        return signature.to_string();
    }

    let Some(name) = swift_member_name(node, source) else {
        return signature.to_string();
    };

    let needle = format!(" {name}:");
    let replacement = format!(" {owner}.{name}:");
    signature.replacen(&needle, &replacement, 1)
}

fn parse_swift_extension_owner(signature: &str) -> Option<String> {
    let normalized = signature
        .strip_prefix("public ")
        .or_else(|| signature.strip_prefix("open "))
        .unwrap_or(signature);
    let target = normalized.strip_prefix("extension ")?;
    let owner = target
        .split(':')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(owner.to_string())
}

fn swift_member_name(node: Node<'_>, source: &str) -> Option<String> {
    let text = node
        .child_by_field_name("name")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)?;
    let name = text
        .split_whitespace()
        .last()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(name.to_string())
}
