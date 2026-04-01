// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::super::{
    children_by_field_name, extract_tree_sitter_prefix_before_fields,
    extract_tree_sitter_prefix_signature, is_exported_tree_sitter_name, named_children, node_text,
    normalize_tree_sitter_signature,
};

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();

    for child in named_children(root) {
        match child.kind() {
            "function_declaration" | "method_declaration"
                if is_exported_tree_sitter_name(child, source) =>
            {
                if let Some(signature) = extract_tree_sitter_prefix_signature(child, source, "body")
                {
                    signatures.push(format!("go:{signature}"));
                }
            }
            "type_declaration" => {
                signatures.extend(extract_exported_type_signatures(child, source))
            }
            "const_declaration" => {
                signatures.extend(extract_exported_const_signatures(child, source))
            }
            "var_declaration" => signatures.extend(extract_exported_var_signatures(child, source)),
            _ => {}
        }
    }

    signatures
}

fn extract_exported_type_signatures(node: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();

    for child in named_children(node) {
        match child.kind() {
            "type_spec" => signatures.extend(extract_go_type_spec_signatures(child, source)),
            "type_alias" => {
                if let Some(signature) = extract_go_type_alias_signature(child, source) {
                    signatures.push(format!("go:{signature}"));
                }
            }
            _ => {}
        }
    }

    signatures
}

fn extract_go_type_spec_signatures(node: Node<'_>, source: &str) -> Vec<String> {
    let Some(name) = go_named_type_name(node, source) else {
        return Vec::new();
    };
    if !is_exported_go_name(&name) {
        return Vec::new();
    }

    let qualified_name = format!("{name}{}", go_type_parameters(node, source));
    let Some(type_node) = node.child_by_field_name("type") else {
        return vec![format!("go:type {qualified_name}")];
    };

    let mut signatures = match type_node.kind() {
        "struct_type" => vec![format!("type {qualified_name} struct")],
        "interface_type" => vec![format!("type {qualified_name} interface")],
        _ => {
            let type_signature = node_text(type_node, source)
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty());
            match type_signature {
                Some(type_signature) => vec![format!("type {qualified_name} {type_signature}")],
                None => vec![format!("type {qualified_name}")],
            }
        }
    };

    if type_node.kind() == "struct_type" {
        signatures.extend(extract_go_struct_field_signatures(
            &qualified_name,
            type_node,
            source,
        ));
    }

    if type_node.kind() == "interface_type" {
        signatures.extend(extract_go_interface_method_signatures(
            &qualified_name,
            type_node,
            source,
        ));
    }

    signatures
        .into_iter()
        .map(|signature| format!("go:{signature}"))
        .collect()
}

fn extract_go_type_alias_signature(node: Node<'_>, source: &str) -> Option<String> {
    let name = go_named_type_name(node, source)?;
    if !is_exported_go_name(&name) {
        return None;
    }

    let aliased_type = node
        .child_by_field_name("type")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())?;

    Some(format!("type {name} = {aliased_type}"))
}

fn extract_go_struct_field_signatures(
    type_name: &str,
    struct_type: Node<'_>,
    source: &str,
) -> Vec<String> {
    let mut signatures = Vec::new();
    let Some(field_list) = named_children(struct_type)
        .into_iter()
        .find(|child| child.kind() == "field_declaration_list")
    else {
        return signatures;
    };

    for field in named_children(field_list) {
        if field.kind() != "field_declaration" {
            continue;
        }

        let name_nodes = children_by_field_name(field, "name");
        let type_text = field
            .child_by_field_name("type")
            .and_then(|child| node_text(child, source))
            .map(normalize_tree_sitter_signature)
            .filter(|value| !value.is_empty());

        if name_nodes.is_empty() {
            if let Some(type_text) =
                extract_tree_sitter_prefix_before_fields(field, source, &["tag"])
            {
                if is_exported_go_embedded_type(&type_text) {
                    signatures.push(format!("embed {type_name} {type_text}"));
                }
            }
            continue;
        }

        for name_node in name_nodes {
            let Some(name) = node_text(name_node, source).map(str::trim) else {
                continue;
            };
            if !is_exported_go_name(name) {
                continue;
            }

            match type_text.as_deref() {
                Some(type_text) => signatures.push(format!("field {type_name}.{name} {type_text}")),
                None => signatures.push(format!("field {type_name}.{name}")),
            }
        }
    }

    signatures
}

fn extract_go_interface_method_signatures(
    type_name: &str,
    interface_type: Node<'_>,
    source: &str,
) -> Vec<String> {
    let mut signatures = Vec::new();

    for child in named_children(interface_type) {
        match child.kind() {
            "method_elem" => {
                let Some(name) = child
                    .child_by_field_name("name")
                    .and_then(|name| node_text(name, source))
                    .map(str::trim)
                else {
                    continue;
                };
                if !is_exported_go_name(name) {
                    continue;
                }

                let parameters = child
                    .child_by_field_name("parameters")
                    .and_then(|value| node_text(value, source))
                    .map(normalize_tree_sitter_signature)
                    .unwrap_or_else(|| "()".to_string());
                let result = child
                    .child_by_field_name("result")
                    .and_then(|value| node_text(value, source))
                    .map(normalize_tree_sitter_signature)
                    .filter(|value| !value.is_empty())
                    .map(|value| format!(" {value}"))
                    .unwrap_or_default();

                signatures.push(format!("interface {type_name}.{name}{parameters}{result}"));
            }
            "type_elem" => {
                if let Some(type_signature) = node_text(child, source)
                    .map(normalize_tree_sitter_signature)
                    .filter(|value| !value.is_empty())
                {
                    signatures.push(format!("interface-type {type_name} {type_signature}"));
                }
            }
            _ => {}
        }
    }

    signatures
}

fn extract_exported_const_signatures(node: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();

    for child in named_children(node) {
        if child.kind() != "const_spec" {
            continue;
        }

        let type_text = child
            .child_by_field_name("type")
            .and_then(|value| node_text(value, source))
            .map(normalize_tree_sitter_signature)
            .filter(|value| !value.is_empty());

        for name_node in children_by_field_name(child, "name") {
            let Some(name) = node_text(name_node, source).map(str::trim) else {
                continue;
            };
            if !is_exported_go_name(name) {
                continue;
            }

            match type_text.as_deref() {
                Some(type_text) => signatures.push(format!("go:const {name} {type_text}")),
                None => signatures.push(format!("go:const {name}")),
            }
        }
    }

    signatures
}

fn extract_exported_var_signatures(node: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();

    for child in named_children(node) {
        match child.kind() {
            "var_spec" => signatures.extend(extract_go_var_spec_signatures(child, source)),
            "var_spec_list" => {
                for spec in named_children(child) {
                    if spec.kind() == "var_spec" {
                        signatures.extend(extract_go_var_spec_signatures(spec, source));
                    }
                }
            }
            _ => {}
        }
    }

    signatures
}

fn extract_go_var_spec_signatures(node: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    let type_text = node
        .child_by_field_name("type")
        .and_then(|value| node_text(value, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty());

    for name_node in children_by_field_name(node, "name") {
        let Some(name) = node_text(name_node, source).map(str::trim) else {
            continue;
        };
        if !is_exported_go_name(name) {
            continue;
        }

        match type_text.as_deref() {
            Some(type_text) => signatures.push(format!("go:var {name} {type_text}")),
            None => signatures.push(format!("go:var {name}")),
        }
    }

    signatures
}

fn go_named_type_name(node: Node<'_>, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
}

fn go_type_parameters(node: Node<'_>, source: &str) -> String {
    node.child_by_field_name("type_parameters")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
}

fn is_exported_go_name(name: &str) -> bool {
    name.chars()
        .next()
        .map(|first| first.is_ascii_uppercase())
        .unwrap_or(false)
}

fn is_exported_go_embedded_type(type_text: &str) -> bool {
    let type_name = type_text
        .trim_start_matches('*')
        .split('[')
        .next()
        .unwrap_or(type_text)
        .rsplit('.')
        .next()
        .unwrap_or(type_text)
        .trim();
    is_exported_go_name(type_name)
}
