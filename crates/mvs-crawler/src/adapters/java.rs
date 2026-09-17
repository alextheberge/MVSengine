// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::super::{
    children_by_field_name, contains_signature_keyword, extract_tree_sitter_prefix_before_fields,
    extract_tree_sitter_prefix_signature, has_tree_sitter_keyword, named_children, node_text,
    normalize_tree_sitter_signature, trim_signature_to_keywords,
};

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    let namespace = java_package_namespace(root, source);
    collect_public_api(root, source, &mut signatures, &namespace, false);
    signatures
}

fn collect_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    type_namespace: &[String],
    inside_interface_like: bool,
) {
    for child in named_children(node) {
        match child.kind() {
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "annotation_type_declaration" => {
                let next_inside_interface_like = matches!(
                    child.kind(),
                    "interface_declaration" | "annotation_type_declaration"
                );
                if let Some(signature) = extract_java_type_signature(
                    child,
                    source,
                    type_namespace,
                    inside_interface_like,
                ) {
                    signatures.push(format!("java:{signature}"));
                }

                if let Some(body) = child.child_by_field_name("body") {
                    let next_namespace = extend_java_type_namespace(type_namespace, child, source);
                    collect_public_api(
                        body,
                        source,
                        signatures,
                        &next_namespace,
                        next_inside_interface_like,
                    );
                }
            }
            "method_declaration" => {
                if let Some(signature) = extract_java_method_signature(
                    child,
                    source,
                    type_namespace,
                    inside_interface_like,
                ) {
                    signatures.push(format!("java:{signature}"));
                }
            }
            "field_declaration" => {
                signatures.extend(extract_java_field_signatures(child, source, type_namespace))
            }
            "constant_declaration" => signatures.extend(extract_java_constant_signatures(
                child,
                source,
                type_namespace,
                inside_interface_like,
            )),
            "class_body" | "interface_body" | "annotation_type_body" | "enum_body_declarations" => {
                collect_public_api(
                    child,
                    source,
                    signatures,
                    type_namespace,
                    inside_interface_like,
                );
            }
            _ => {}
        }
    }
}

fn extend_java_type_namespace(namespace: &[String], node: Node<'_>, source: &str) -> Vec<String> {
    let mut next = namespace.to_vec();
    let Some(name) = java_declaration_name(node, source) else {
        return next;
    };
    next.push(name);
    next
}

fn java_package_namespace(root: Node<'_>, source: &str) -> Vec<String> {
    named_children(root)
        .into_iter()
        .find(|child| child.kind() == "package_declaration")
        .and_then(|child| java_package_name(child, source))
        .map(|qualified| split_qualified_path(&qualified))
        .unwrap_or_default()
}

fn java_package_name(node: Node<'_>, source: &str) -> Option<String> {
    named_children(node)
        .into_iter()
        .find(|child| matches!(child.kind(), "identifier" | "scoped_identifier"))
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
}

fn java_declaration_name(node: Node<'_>, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
}

fn extract_java_type_signature(
    node: Node<'_>,
    source: &str,
    namespace: &[String],
    inside_interface_like: bool,
) -> Option<String> {
    let raw = extract_tree_sitter_prefix_signature(node, source, "body")?;
    let signature = if has_tree_sitter_keyword(node, source, "body", "public") {
        trim_signature_to_keywords(&raw, &["public"])?
    } else if inside_interface_like {
        format!("public {}", normalize_tree_sitter_signature(&raw))
    } else {
        return None;
    };

    Some(format!(
        "type {}",
        qualify_java_type_signature(&signature, node, source, namespace)
    ))
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

    let Some(name) = java_declaration_name(node, source) else {
        return signature.to_string();
    };

    qualify_named_signature(signature, &name, &format!("{owner}.{name}"))
}

fn qualify_java_type_signature(
    signature: &str,
    node: Node<'_>,
    source: &str,
    namespace: &[String],
) -> String {
    let Some(name) = java_declaration_name(node, source) else {
        return signature.to_string();
    };
    let qualified_name = qualify_name(namespace, &name);
    if qualified_name == name {
        return signature.to_string();
    }

    qualify_named_signature(signature, &name, &qualified_name)
}

fn qualify_named_signature(signature: &str, name: &str, qualified_name: &str) -> String {
    let needle = format!(" {name}(");
    if signature.contains(&needle) {
        return signature.replacen(&needle, &format!(" {qualified_name}("), 1);
    }

    let needle = format!(" {name}");
    signature.replacen(&needle, &format!(" {qualified_name}"), 1)
}

fn extract_java_method_signature(
    node: Node<'_>,
    source: &str,
    type_namespace: &[String],
    inside_interface_like: bool,
) -> Option<String> {
    let raw = extract_tree_sitter_prefix_signature(node, source, "body")?;
    let signature = if has_tree_sitter_keyword(node, source, "body", "public") {
        trim_signature_to_keywords(&raw, &["public"])?
    } else if inside_interface_like && !has_tree_sitter_keyword(node, source, "body", "private") {
        format!("public {}", normalize_tree_sitter_signature(&raw))
    } else {
        return None;
    };

    Some(format!(
        "method {}",
        qualify_java_method_signature(&signature, node, source, type_namespace)
    ))
}

fn extract_java_field_signatures(
    node: Node<'_>,
    source: &str,
    type_namespace: &[String],
) -> Vec<String> {
    let Some(owner) = java_owner_name(type_namespace) else {
        return Vec::new();
    };
    let Some(modifiers) = extract_tree_sitter_prefix_before_fields(node, source, &["type"]) else {
        return Vec::new();
    };
    if !contains_signature_keyword(&modifiers, "public") {
        return Vec::new();
    }

    let Some(type_signature) = java_declared_type_signature(node, source) else {
        return Vec::new();
    };

    let mut prefix_parts = vec!["field".to_string(), "public".to_string()];
    if contains_signature_keyword(&modifiers, "static") {
        prefix_parts.push("static".to_string());
    }
    if contains_signature_keyword(&modifiers, "final") {
        prefix_parts.push("final".to_string());
    }
    prefix_parts.push(type_signature);

    extract_java_declarator_signatures(node, source, &owner, &prefix_parts)
}

fn extract_java_constant_signatures(
    node: Node<'_>,
    source: &str,
    type_namespace: &[String],
    inside_interface_like: bool,
) -> Vec<String> {
    let Some(owner) = java_owner_name(type_namespace) else {
        return Vec::new();
    };
    let Some(type_signature) = java_declared_type_signature(node, source) else {
        return Vec::new();
    };
    let modifiers =
        extract_tree_sitter_prefix_before_fields(node, source, &["type"]).unwrap_or_default();
    if !inside_interface_like && !contains_signature_keyword(&modifiers, "public") {
        return Vec::new();
    }

    let mut prefix_parts = vec!["const".to_string(), "public".to_string()];
    if inside_interface_like || contains_signature_keyword(&modifiers, "static") {
        prefix_parts.push("static".to_string());
    }
    if inside_interface_like || contains_signature_keyword(&modifiers, "final") {
        prefix_parts.push("final".to_string());
    }
    prefix_parts.push(type_signature);

    extract_java_declarator_signatures(node, source, &owner, &prefix_parts)
}

fn extract_java_declarator_signatures(
    node: Node<'_>,
    source: &str,
    owner: &str,
    prefix_parts: &[String],
) -> Vec<String> {
    let mut signatures = Vec::new();
    for declarator in children_by_field_name(node, "declarator") {
        let Some(name) = declarator
            .child_by_field_name("name")
            .and_then(|child| node_text(child, source))
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let dimensions = declarator
            .child_by_field_name("dimensions")
            .and_then(|child| node_text(child, source))
            .map(normalize_tree_sitter_signature)
            .unwrap_or_default();

        let mut parts = prefix_parts.to_vec();
        if dimensions.is_empty() {
            parts.push(format!("{owner}.{name}"));
        } else {
            let Some(type_signature) = parts.pop() else {
                continue;
            };
            parts.push(format!("{type_signature}{dimensions}"));
            parts.push(format!("{owner}.{name}"));
        }
        signatures.push(format!("java:{}", parts.join(" ")));
    }
    signatures
}

fn java_declared_type_signature(node: Node<'_>, source: &str) -> Option<String> {
    node.child_by_field_name("type")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
}

fn java_owner_name(type_namespace: &[String]) -> Option<String> {
    if type_namespace.is_empty() {
        None
    } else {
        Some(type_namespace.join("."))
    }
}

fn qualify_name(namespace: &[String], name: &str) -> String {
    if namespace.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", namespace.join("."), name)
    }
}

fn split_qualified_path(value: &str) -> Vec<String> {
    value
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(ToString::to_string)
        .collect()
}
