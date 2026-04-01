// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::super::{
    contains_any_signature_keyword, extract_tree_sitter_prefix_signature, has_tree_sitter_keyword,
    named_children, node_text, normalize_tree_sitter_signature, trim_signature_to_keywords,
};

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    collect_public_api(root, source, &mut signatures, &[], false);
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
            | "struct_declaration" => {
                let next_inside_interface_like = child.kind() == "interface_declaration";
                if let Some(signature) =
                    extract_csharp_type_signature(child, source, inside_interface_like)
                {
                    signatures.push(format!("csharp:{signature}"));
                }

                if let Some(body) = child.child_by_field_name("body") {
                    let next_namespace =
                        extend_csharp_type_namespace(type_namespace, child, source);
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
                if let Some(signature) = extract_csharp_method_signature(
                    child,
                    source,
                    type_namespace,
                    inside_interface_like,
                ) {
                    signatures.push(format!("csharp:{signature}"));
                }
            }
            "property_declaration" => signatures.extend(extract_csharp_property_signatures(
                child,
                source,
                type_namespace,
                inside_interface_like,
            )),
            "field_declaration" => signatures.extend(extract_csharp_field_signatures(
                child,
                source,
                type_namespace,
            )),
            "namespace_declaration" => {
                if let Some(body) = child.child_by_field_name("body") {
                    collect_public_api(
                        body,
                        source,
                        signatures,
                        type_namespace,
                        inside_interface_like,
                    );
                }
            }
            "compilation_unit" | "declaration_list" | "file_scoped_namespace_declaration" => {
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

fn extend_csharp_type_namespace(namespace: &[String], node: Node<'_>, source: &str) -> Vec<String> {
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

fn qualify_csharp_method_signature(
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

fn extract_csharp_type_signature(
    node: Node<'_>,
    source: &str,
    inside_interface_like: bool,
) -> Option<String> {
    let raw = extract_tree_sitter_prefix_signature(node, source, "body")?;
    let signature = if has_tree_sitter_keyword(node, source, "body", "public") {
        trim_signature_to_keywords(&raw, &["public"])?
    } else if inside_interface_like
        && !contains_any_signature_keyword(&raw, &["private", "protected", "internal"])
    {
        format!("public {}", normalize_tree_sitter_signature(&raw))
    } else {
        return None;
    };

    Some(format!("type {signature}"))
}

fn extract_csharp_method_signature(
    node: Node<'_>,
    source: &str,
    type_namespace: &[String],
    inside_interface_like: bool,
) -> Option<String> {
    if csharp_has_child_kind(node, "explicit_interface_specifier") {
        return None;
    }

    let raw = extract_tree_sitter_prefix_signature(node, source, "body")?;
    let signature = if has_tree_sitter_keyword(node, source, "body", "public") {
        trim_signature_to_keywords(&raw, &["public"])?
    } else if inside_interface_like
        && !contains_any_signature_keyword(&raw, &["private", "protected", "internal"])
    {
        format!("public {}", normalize_tree_sitter_signature(&raw))
    } else {
        return None;
    };

    Some(format!(
        "method {}",
        qualify_csharp_method_signature(&signature, node, source, type_namespace)
    ))
}

fn extract_csharp_property_signatures(
    node: Node<'_>,
    source: &str,
    type_namespace: &[String],
    inside_interface_like: bool,
) -> Vec<String> {
    if csharp_has_child_kind(node, "explicit_interface_specifier") {
        return Vec::new();
    }

    let is_public = csharp_has_modifier(node, source, "public")
        || (inside_interface_like
            && !csharp_has_any_modifier(node, source, &["private", "protected", "internal"]));
    if !is_public {
        return Vec::new();
    }
    let Some(owner) = csharp_owner_name_opt(type_namespace) else {
        return Vec::new();
    };

    let Some(type_signature) = node
        .child_by_field_name("type")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
    else {
        return Vec::new();
    };
    let Some(name) = node
        .child_by_field_name("name")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
    else {
        return Vec::new();
    };

    let accessors = extract_csharp_public_accessors(node, source, inside_interface_like);
    if accessors.is_empty() && node.child_by_field_name("value").is_none() {
        return Vec::new();
    }

    let mut parts = vec!["property".to_string(), "public".to_string()];
    parts.extend(csharp_non_visibility_modifiers(node, source, &[]));
    parts.push(type_signature);
    parts.push(format!("{owner}.{name}"));
    if accessors.is_empty() {
        parts.push("{ get }".to_string());
    } else {
        parts.push(format!("{{ {} }}", accessors.join("; ")));
    }

    vec![format!("csharp:{}", parts.join(" "))]
}

fn extract_csharp_field_signatures(
    node: Node<'_>,
    source: &str,
    type_namespace: &[String],
) -> Vec<String> {
    if !csharp_has_modifier(node, source, "public") {
        return Vec::new();
    }

    let Some(owner) = csharp_owner_name_opt(type_namespace) else {
        return Vec::new();
    };
    let Some(variable_declaration) = named_children(node)
        .into_iter()
        .find(|child| child.kind() == "variable_declaration")
    else {
        return Vec::new();
    };
    let Some(type_signature) = variable_declaration
        .child_by_field_name("type")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
    else {
        return Vec::new();
    };

    let kind = if csharp_has_modifier(node, source, "const") {
        "const"
    } else {
        "field"
    };

    let mut prefix_parts = vec![kind.to_string(), "public".to_string()];
    prefix_parts.extend(csharp_non_visibility_modifiers(node, source, &["const"]));
    prefix_parts.push(type_signature);

    let mut signatures = Vec::new();
    for declarator in named_children(variable_declaration) {
        if declarator.kind() != "variable_declarator" {
            continue;
        }
        let Some(name) = declarator
            .child_by_field_name("name")
            .and_then(|child| node_text(child, source))
            .map(normalize_tree_sitter_signature)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };

        let mut parts = prefix_parts.clone();
        parts.push(format!("{owner}.{name}"));
        signatures.push(format!("csharp:{}", parts.join(" ")));
    }

    signatures
}

fn extract_csharp_public_accessors(
    node: Node<'_>,
    source: &str,
    _inside_interface_like: bool,
) -> Vec<String> {
    let Some(accessors) = node.child_by_field_name("accessors") else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for accessor in named_children(accessors) {
        if accessor.kind() != "accessor_declaration" {
            continue;
        }

        let is_public = csharp_has_modifier(accessor, source, "public")
            || !csharp_has_any_modifier(accessor, source, &["private", "protected", "internal"]);
        if !is_public {
            continue;
        }

        let Some(name) = accessor
            .child_by_field_name("name")
            .and_then(|child| node_text(child, source))
            .map(normalize_tree_sitter_signature)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        names.push(name);
    }

    names
}

fn csharp_non_visibility_modifiers(node: Node<'_>, source: &str, ignored: &[&str]) -> Vec<String> {
    named_children(node)
        .into_iter()
        .filter(|child| child.kind() == "modifier")
        .filter_map(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| {
            !value.is_empty()
                && !matches!(
                    value.as_str(),
                    "public" | "private" | "protected" | "internal"
                )
                && !ignored.contains(&value.as_str())
        })
        .collect()
}

fn csharp_has_modifier(node: Node<'_>, source: &str, modifier: &str) -> bool {
    named_children(node).into_iter().any(|child| {
        child.kind() == "modifier"
            && node_text(child, source)
                .map(normalize_tree_sitter_signature)
                .as_deref()
                == Some(modifier)
    })
}

fn csharp_has_any_modifier(node: Node<'_>, source: &str, modifiers: &[&str]) -> bool {
    modifiers
        .iter()
        .any(|modifier| csharp_has_modifier(node, source, modifier))
}

fn csharp_has_child_kind(node: Node<'_>, kind: &str) -> bool {
    named_children(node)
        .into_iter()
        .any(|child| child.kind() == kind)
}

fn csharp_owner_name(type_namespace: &[String]) -> String {
    type_namespace.join(".")
}

fn csharp_owner_name_opt(type_namespace: &[String]) -> Option<String> {
    if type_namespace.is_empty() {
        None
    } else {
        Some(csharp_owner_name(type_namespace))
    }
}
