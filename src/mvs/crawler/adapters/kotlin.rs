// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::super::{
    contains_any_signature_keyword, extract_tree_sitter_prefix_before_named_child, named_children,
    node_text, normalize_kotlin_signature, normalize_tree_sitter_signature,
};

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    let namespace = kotlin_package_namespace(root, source);
    collect_public_api(root, source, &mut signatures, &namespace);
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
                if let Some(signature) =
                    extract_kotlin_type_signature(child, source, type_namespace)
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
            "property_declaration" => signatures.extend(extract_kotlin_property_signatures(
                child,
                source,
                type_namespace,
            )),
            "class_body" | "enum_class_body" => {
                collect_public_api(child, source, signatures, type_namespace)
            }
            _ => {}
        }
    }
}

fn extend_kotlin_type_namespace(namespace: &[String], node: Node<'_>, source: &str) -> Vec<String> {
    let mut next = namespace.to_vec();
    let Some(name) = kotlin_declaration_name(node, source) else {
        return next;
    };
    next.push(name);
    next
}

fn kotlin_package_namespace(root: Node<'_>, source: &str) -> Vec<String> {
    named_children(root)
        .into_iter()
        .find(|child| child.kind() == "package_header")
        .and_then(|child| kotlin_package_name(child, source))
        .map(|qualified| split_qualified_path(&qualified))
        .unwrap_or_default()
}

fn kotlin_package_name(node: Node<'_>, source: &str) -> Option<String> {
    named_children(node)
        .into_iter()
        .find(|child| child.kind() == "qualified_identifier")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
}

fn kotlin_declaration_name(node: Node<'_>, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
}

fn extract_kotlin_type_signature(
    node: Node<'_>,
    source: &str,
    namespace: &[String],
) -> Option<String> {
    let signature = extract_tree_sitter_prefix_before_named_child(
        node,
        source,
        &["class_body", "enum_class_body"],
    )
    .and_then(|value| normalize_kotlin_signature(&value))?;

    Some(qualify_kotlin_type_signature(
        &signature, node, source, namespace,
    ))
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

    let Some(name) = kotlin_declaration_name(node, source) else {
        return signature.to_string();
    };

    qualify_named_signature(signature, &name, &format!("{owner}.{name}"))
}

fn qualify_kotlin_type_signature(
    signature: &str,
    node: Node<'_>,
    source: &str,
    namespace: &[String],
) -> String {
    let Some(name) = kotlin_declaration_name(node, source) else {
        return signature.to_string();
    };
    let qualified_name = qualify_name(namespace, &name);
    if qualified_name == name {
        return signature.to_string();
    }

    qualify_named_signature(signature, &name, &qualified_name)
}

fn extract_kotlin_property_signatures(
    node: Node<'_>,
    source: &str,
    type_namespace: &[String],
) -> Vec<String> {
    let Some(prefix) = extract_tree_sitter_prefix_before_named_child(
        node,
        source,
        &[
            "getter",
            "setter",
            "property_delegate",
            "expression",
            "type_constraints",
        ],
    ) else {
        return Vec::new();
    };

    if contains_any_signature_keyword(&prefix, &["private", "protected", "internal"]) {
        return Vec::new();
    }

    let Some(variable) = named_children(node)
        .into_iter()
        .find(|child| child.kind() == "variable_declaration")
    else {
        return Vec::new();
    };
    let Some(name) = kotlin_variable_name(variable, source) else {
        return Vec::new();
    };

    let mut kind_parts = Vec::new();
    if contains_keyword(&prefix, "const") {
        kind_parts.push("const");
    }
    let property_kind = if contains_keyword(&prefix, "var") {
        "var"
    } else if contains_keyword(&prefix, "val") {
        "val"
    } else {
        return Vec::new();
    };
    kind_parts.push(property_kind);

    let qualified_name = if type_namespace.is_empty() {
        name
    } else {
        format!("{}.{}", type_namespace.join("."), name)
    };

    let mut signature = format!("{} {}", kind_parts.join(" "), qualified_name);
    if let Some(type_signature) = kotlin_variable_type(variable, source) {
        signature.push(':');
        signature.push(' ');
        signature.push_str(&type_signature);
    }

    vec![format!("kotlin:{signature}")]
}

fn kotlin_variable_name(node: Node<'_>, source: &str) -> Option<String> {
    named_children(node)
        .into_iter()
        .find(|child| child.kind() == "identifier")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
}

fn kotlin_variable_type(node: Node<'_>, source: &str) -> Option<String> {
    named_children(node)
        .into_iter()
        .find(|child| child.kind() != "annotation" && child.kind() != "identifier")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
}

fn contains_keyword(signature: &str, keyword: &str) -> bool {
    signature
        .split(|char: char| !char.is_ascii_alphanumeric() && char != '_')
        .any(|token| token == keyword)
}

fn qualify_named_signature(signature: &str, name: &str, qualified_name: &str) -> String {
    let needle = format!(" {name}(");
    if signature.contains(&needle) {
        return signature.replacen(&needle, &format!(" {qualified_name}("), 1);
    }

    let needle = format!(" {name}");
    signature.replacen(&needle, &format!(" {qualified_name}"), 1)
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
