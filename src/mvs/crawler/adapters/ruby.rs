// SPDX-License-Identifier: AGPL-3.0-only
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
    collect_public_api(root, source, &mut signatures, Visibility::Public, None);
    signatures
}

fn collect_public_api(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    visibility: Visibility,
    singleton_receiver: Option<&str>,
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
                    current_visibility,
                    singleton_receiver,
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
                collect_public_api(body, source, signatures, Visibility::Public, None);
            }
        }
        "module" => {
            if let Some(signature) = extract_tree_sitter_prefix_signature(node, source, "body")
                .and_then(|value| normalize_ruby_type_signature(&value, "module"))
            {
                signatures.push(format!("ruby:{signature}"));
            }

            if let Some(body) = node.child_by_field_name("body") {
                collect_public_api(body, source, signatures, Visibility::Public, None);
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
                    Visibility::Public,
                    receiver.as_deref().or(singleton_receiver),
                );
            }
        }
        "begin" | "if" | "unless" | "case" | "when" | "for" | "while" | "until" | "rescue" => {
            for child in named_children(node) {
                collect_public_api(child, source, signatures, visibility, singleton_receiver);
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
