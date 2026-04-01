// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::HashSet;

use tree_sitter::Node;

use super::super::{
    extract_tree_sitter_prefix_signature, named_children, node_text,
    normalize_tree_sitter_signature, trim_signature_to_keywords,
};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Visibility {
    Public,
    Protected,
    Private,
}

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    let mut hidden_constants = HashSet::new();
    collect_public_api(
        root,
        source,
        &mut signatures,
        &mut hidden_constants,
        Visibility::Public,
        None,
        &[],
    );
    signatures
}

fn collect_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    hidden_constants: &mut HashSet<String>,
    visibility: Visibility,
    singleton_receiver: Option<&str>,
    namespace: &[String],
) {
    match node.kind() {
        "program" | "body_statement" => {
            let mut current_visibility = visibility;
            for child in named_children(node) {
                if let Some(next_visibility) = visibility_change(child, source) {
                    current_visibility = next_visibility;
                    continue;
                }

                collect_public_api(
                    child,
                    source,
                    signatures,
                    hidden_constants,
                    current_visibility,
                    singleton_receiver,
                    namespace,
                );
            }
        }
        "class" => {
            if let Some(signature) = extract_tree_sitter_prefix_signature(node, source, "body")
                .and_then(|value| normalize_ruby_type_signature(&value, "class"))
            {
                signatures.push(format!("ruby:{signature}"));
            }

            if let Some(body) = node.child_by_field_name("body") {
                let next_namespace = extend_ruby_namespace(namespace, node, source);
                collect_public_api(
                    body,
                    source,
                    signatures,
                    hidden_constants,
                    Visibility::Public,
                    None,
                    &next_namespace,
                );
            }
        }
        "module" => {
            if let Some(signature) = extract_tree_sitter_prefix_signature(node, source, "body")
                .and_then(|value| normalize_ruby_type_signature(&value, "module"))
            {
                signatures.push(format!("ruby:{signature}"));
            }

            if let Some(body) = node.child_by_field_name("body") {
                let next_namespace = extend_ruby_namespace(namespace, node, source);
                collect_public_api(
                    body,
                    source,
                    signatures,
                    hidden_constants,
                    Visibility::Public,
                    None,
                    &next_namespace,
                );
            }
        }
        "assignment" => {
            if let Some(signature) =
                extract_ruby_constant_signature(node, source, namespace, hidden_constants)
            {
                signatures.push(format!("ruby:{signature}"));
            }
        }
        "call" => {
            hide_ruby_private_constants(node, source, signatures, hidden_constants, namespace);

            if visibility == Visibility::Public && singleton_receiver.is_none() {
                signatures.extend(
                    extract_ruby_attribute_signatures(node, source)
                        .into_iter()
                        .map(|signature| format!("ruby:{signature}")),
                );
            }
        }
        "method" => {
            if visibility == Visibility::Public {
                if let Some(signature) =
                    normalize_ruby_method_signature(node, source, singleton_receiver)
                {
                    signatures.push(format!("ruby:{signature}"));
                }
            }
        }
        "singleton_method" => {
            if visibility == Visibility::Public {
                if let Some(signature) = extract_tree_sitter_prefix_signature(node, source, "body")
                    .and_then(|value| trim_signature_to_keywords(&value, &["def"]))
                {
                    signatures.push(format!("ruby:{signature}"));
                }
            }
        }
        "singleton_class" => {
            if let Some(body) = node.child_by_field_name("body") {
                let receiver = node
                    .child_by_field_name("value")
                    .and_then(|value| node_text(value, source))
                    .map(normalize_tree_sitter_signature);
                collect_public_api(
                    body,
                    source,
                    signatures,
                    hidden_constants,
                    Visibility::Public,
                    receiver.as_deref().or(singleton_receiver),
                    namespace,
                );
            }
        }
        "begin" | "if" | "unless" | "case" | "when" | "for" | "while" | "until" | "rescue" => {
            for child in named_children(node) {
                collect_public_api(
                    child,
                    source,
                    signatures,
                    hidden_constants,
                    visibility,
                    singleton_receiver,
                    namespace,
                );
            }
        }
        _ => {}
    }
}

fn normalize_ruby_method_signature(
    node: Node<'_>,
    source: &str,
    singleton_receiver: Option<&str>,
) -> Option<String> {
    let signature = extract_tree_sitter_prefix_signature(node, source, "body")
        .and_then(|value| trim_signature_to_keywords(&value, &["def"]))?;
    let Some(receiver) = singleton_receiver else {
        return Some(signature);
    };

    let rest = signature.strip_prefix("def ")?;
    Some(normalize_tree_sitter_signature(&format!(
        "def {receiver}.{}",
        rest.trim_start()
    )))
}

fn visibility_change(node: Node<'_>, source: &str) -> Option<Visibility> {
    if let Some(text) = node_text(node, source)
        .map(normalize_tree_sitter_signature)
        .filter(|text| matches!(text.as_str(), "public" | "protected" | "private"))
    {
        return match text.as_str() {
            "public" => Some(Visibility::Public),
            "protected" => Some(Visibility::Protected),
            "private" => Some(Visibility::Private),
            _ => None,
        };
    }

    if node.kind() != "call" || node.child_by_field_name("receiver").is_some() {
        return None;
    }

    let method = node
        .child_by_field_name("method")
        .and_then(|value| node_text(value, source))
        .map(str::trim)?;

    match method {
        "public" => Some(Visibility::Public),
        "protected" => Some(Visibility::Protected),
        "private" => Some(Visibility::Private),
        _ => None,
    }
}

fn normalize_ruby_type_signature(signature: &str, keyword: &str) -> Option<String> {
    let normalized = trim_signature_to_keywords(signature, &[keyword])?;
    if keyword == "class" {
        if let Some((name, superclass)) = normalized.split_once('<') {
            return Some(format!("{} < {}", name.trim_end(), superclass.trim_start()));
        }
    }

    Some(normalized)
}

fn extend_ruby_namespace(namespace: &[String], node: Node<'_>, source: &str) -> Vec<String> {
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

fn extract_ruby_constant_signature(
    node: Node<'_>,
    source: &str,
    namespace: &[String],
    hidden_constants: &HashSet<String>,
) -> Option<String> {
    let left = node.child_by_field_name("left")?;
    let constant_name = match left.kind() {
        "constant" => {
            let name = node_text(left, source)
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())?;
            if namespace.is_empty() {
                name
            } else {
                format!("{}::{name}", namespace.join("::"))
            }
        }
        "scope_resolution" => node_text(left, source)
            .map(normalize_tree_sitter_signature)
            .filter(|value| !value.is_empty())?,
        _ => return None,
    };
    if hidden_constants.contains(&constant_name) {
        return None;
    }

    Some(format!("const {constant_name}"))
}

fn hide_ruby_private_constants(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    hidden_constants: &mut HashSet<String>,
    namespace: &[String],
) {
    for constant_name in extract_ruby_private_constants(node, source, namespace) {
        hidden_constants.insert(constant_name.clone());
        let hidden_signature = format!("ruby:const {constant_name}");
        signatures.retain(|signature| signature != &hidden_signature);
    }
}

fn extract_ruby_private_constants(
    node: Node<'_>,
    source: &str,
    namespace: &[String],
) -> Vec<String> {
    if node.child_by_field_name("receiver").is_some() {
        return Vec::new();
    }

    let Some(method) = ruby_call_method_name(node, source) else {
        return Vec::new();
    };
    if method != "private_constant" {
        return Vec::new();
    }

    ruby_call_argument_names(node, source)
        .into_iter()
        .map(|name| qualify_ruby_constant_name(namespace, &name))
        .collect()
}

fn extract_ruby_attribute_signatures(node: Node<'_>, source: &str) -> Vec<String> {
    if node.child_by_field_name("receiver").is_some() {
        return Vec::new();
    }

    let Some(method) = ruby_call_method_name(node, source) else {
        return Vec::new();
    };
    if !matches!(
        method.as_str(),
        "attr_reader" | "attr_writer" | "attr_accessor"
    ) {
        return Vec::new();
    }

    ruby_call_argument_names(node, source)
        .into_iter()
        .map(|name| format!("{method} {name}"))
        .collect()
}

fn ruby_call_method_name(node: Node<'_>, source: &str) -> Option<String> {
    node.child_by_field_name("method")
        .and_then(|value| node_text(value, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
}

fn ruby_call_argument_names(node: Node<'_>, source: &str) -> Vec<String> {
    let Some(arguments) = node.child_by_field_name("arguments") else {
        return Vec::new();
    };

    let mut names = Vec::new();
    collect_ruby_call_argument_names(arguments, source, &mut names);
    names
}

fn collect_ruby_call_argument_names(node: Node<'_>, source: &str, names: &mut Vec<String>) {
    match node.kind() {
        "simple_symbol" | "delimited_symbol" | "bare_symbol" | "constant" | "string_content" => {
            if let Some(name) = normalize_ruby_call_argument_name(node, source) {
                names.push(name);
            }
            return;
        }
        _ => {}
    }

    for child in named_children(node) {
        collect_ruby_call_argument_names(child, source, names);
    }
}

fn normalize_ruby_call_argument_name(node: Node<'_>, source: &str) -> Option<String> {
    let name = node_text(node, source)?
        .trim()
        .trim_start_matches(':')
        .trim_matches('"')
        .trim_matches('\'');
    let normalized = normalize_tree_sitter_signature(name);
    (!normalized.is_empty()).then_some(normalized)
}

fn qualify_ruby_constant_name(namespace: &[String], name: &str) -> String {
    if name.contains("::") || namespace.is_empty() {
        name.to_string()
    } else {
        format!("{}::{name}", namespace.join("::"))
    }
}
