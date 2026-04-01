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

#[derive(Default)]
struct RubyExportState {
    declared_constants: HashSet<String>,
    hidden_constants: HashSet<String>,
    hidden_singleton_methods: HashSet<String>,
    module_function_methods: HashSet<String>,
    module_function_all_owners: HashSet<String>,
    extend_self_owners: HashSet<String>,
}

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    let mut state = RubyExportState::default();
    collect_public_api(
        root,
        source,
        &mut signatures,
        &mut state,
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
    state: &mut RubyExportState,
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
                    state,
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
                    state,
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
                    state,
                    Visibility::Public,
                    None,
                    &next_namespace,
                );
            }
        }
        "assignment" => {
            if let Some(constant_name) = extract_ruby_constant_name(node, source, namespace) {
                state.declared_constants.insert(constant_name.clone());
                if !state.hidden_constants.contains(&constant_name) {
                    push_unique(signatures, format!("ruby:const {constant_name}"));
                }
            }
        }
        "call" => {
            apply_ruby_constant_visibility(node, source, signatures, state, namespace);
            apply_ruby_singleton_export_controls(
                node,
                source,
                signatures,
                state,
                singleton_receiver,
                namespace,
            );

            if visibility == Visibility::Public && singleton_receiver.is_none() {
                signatures.extend(
                    extract_ruby_attribute_signatures(node, source, namespace)
                        .into_iter()
                        .map(|signature| format!("ruby:{signature}")),
                );
            }
        }
        "method" => {
            if visibility == Visibility::Public {
                push_ruby_method_signature(
                    node,
                    source,
                    signatures,
                    state,
                    singleton_receiver,
                    namespace,
                );
            }
        }
        "singleton_method" => {
            if visibility == Visibility::Public {
                push_ruby_singleton_method_signature(node, source, signatures, state, namespace);
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
                    state,
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
                    state,
                    visibility,
                    singleton_receiver,
                    namespace,
                );
            }
        }
        _ => {}
    }
}

fn push_ruby_method_signature(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    state: &RubyExportState,
    singleton_receiver: Option<&str>,
    namespace: &[String],
) {
    let Some(signature) =
        normalize_ruby_method_signature(node, source, singleton_receiver, namespace)
    else {
        return;
    };
    let method_name = ruby_method_name(node, source);

    if singleton_receiver.is_some() {
        if let (Some(owner), Some(name)) = (
            resolve_ruby_singleton_owner(singleton_receiver, namespace),
            method_name.as_deref(),
        ) {
            if state
                .hidden_singleton_methods
                .contains(&ruby_singleton_method_key(&owner, name))
            {
                return;
            }
        }
        push_unique(signatures, format!("ruby:{signature}"));
        return;
    }

    let Some(owner) = ruby_owner(namespace) else {
        push_unique(signatures, format!("ruby:{signature}"));
        return;
    };
    let Some(name) = method_name else {
        push_unique(signatures, format!("ruby:{signature}"));
        return;
    };

    if ruby_module_function_applies(state, &owner, &name) {
        if state
            .hidden_singleton_methods
            .contains(&ruby_singleton_method_key(&owner, &name))
        {
            return;
        }
        if let Some(singleton_signature) =
            transform_ruby_instance_signature_to_singleton(&signature, &owner)
        {
            push_unique(signatures, format!("ruby:{singleton_signature}"));
        }
        return;
    }

    push_unique(signatures, format!("ruby:{signature}"));

    if state.extend_self_owners.contains(&owner)
        && !state
            .hidden_singleton_methods
            .contains(&ruby_singleton_method_key(&owner, &name))
    {
        if let Some(singleton_signature) =
            transform_ruby_instance_signature_to_singleton(&signature, &owner)
        {
            push_unique(signatures, format!("ruby:{singleton_signature}"));
        }
    }
}

fn push_ruby_singleton_method_signature(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    state: &RubyExportState,
    namespace: &[String],
) {
    let Some(signature) = extract_tree_sitter_prefix_signature(node, source, "body")
        .and_then(|value| trim_signature_to_keywords(&value, &["def"]))
        .map(|value| qualify_ruby_singleton_signature(&value, namespace))
    else {
        return;
    };

    if let Some((owner, name)) = ruby_singleton_signature_metadata(&format!("ruby:{signature}")) {
        if state
            .hidden_singleton_methods
            .contains(&ruby_singleton_method_key(&owner, &name))
        {
            return;
        }
    }

    push_unique(signatures, format!("ruby:{signature}"));
}

fn normalize_ruby_method_signature(
    node: Node<'_>,
    source: &str,
    singleton_receiver: Option<&str>,
    namespace: &[String],
) -> Option<String> {
    let signature = extract_tree_sitter_prefix_signature(node, source, "body")
        .and_then(|value| trim_signature_to_keywords(&value, &["def"]))?;
    let owner = namespace.join("::");
    let Some(receiver) = singleton_receiver else {
        if owner.is_empty() {
            return Some(signature);
        }
        return Some(qualify_ruby_instance_signature(&signature, &owner));
    };

    let resolved_receiver = if receiver == "self" && !owner.is_empty() {
        owner
    } else {
        receiver.to_string()
    };

    Some(qualify_ruby_receiver_signature(
        &signature,
        &resolved_receiver,
    ))
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

fn extract_ruby_constant_name(
    node: Node<'_>,
    source: &str,
    namespace: &[String],
) -> Option<String> {
    let left = node.child_by_field_name("left")?;
    match left.kind() {
        "constant" => {
            let name = node_text(left, source)
                .map(normalize_tree_sitter_signature)
                .filter(|value| !value.is_empty())?;
            if namespace.is_empty() {
                Some(name)
            } else {
                Some(format!("{}::{name}", namespace.join("::")))
            }
        }
        "scope_resolution" => node_text(left, source)
            .map(normalize_tree_sitter_signature)
            .filter(|value| !value.is_empty()),
        _ => None,
    }
}

fn apply_ruby_constant_visibility(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    state: &mut RubyExportState,
    namespace: &[String],
) {
    for constant_name in extract_ruby_private_constants(node, source, namespace) {
        state.hidden_constants.insert(constant_name.clone());
        let hidden_signature = format!("ruby:const {constant_name}");
        signatures.retain(|signature| signature != &hidden_signature);
    }

    for constant_name in extract_ruby_public_constants(node, source, namespace) {
        state.hidden_constants.remove(&constant_name);
        if state.declared_constants.contains(&constant_name) {
            push_unique(signatures, format!("ruby:const {constant_name}"));
        }
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

fn extract_ruby_public_constants(
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
    if method != "public_constant" {
        return Vec::new();
    }

    ruby_call_argument_names(node, source)
        .into_iter()
        .map(|name| qualify_ruby_constant_name(namespace, &name))
        .collect()
}

fn apply_ruby_singleton_export_controls(
    node: Node<'_>,
    source: &str,
    signatures: &mut Vec<String>,
    state: &mut RubyExportState,
    singleton_receiver: Option<&str>,
    namespace: &[String],
) {
    if node.child_by_field_name("receiver").is_some() {
        return;
    }

    let Some(method) = ruby_call_method_name(node, source) else {
        return;
    };

    match method.as_str() {
        "module_function" => {
            let Some(owner) = ruby_owner(namespace) else {
                return;
            };
            let method_names = ruby_call_argument_names(node, source);
            if method_names.is_empty() {
                state.module_function_all_owners.insert(owner);
                return;
            }

            for name in method_names {
                state
                    .module_function_methods
                    .insert(ruby_module_function_key(&owner, &name));
                promote_ruby_module_function(signatures, state, &owner, &name);
            }
        }
        "extend" => {
            let Some(owner) = ruby_owner(namespace) else {
                return;
            };
            if !ruby_call_has_self_argument(node, source) {
                return;
            }

            state.extend_self_owners.insert(owner.clone());
            export_ruby_instance_methods_as_singleton(signatures, state, &owner);
        }
        "private_class_method" => {
            let Some(owner) = resolve_ruby_singleton_owner(singleton_receiver, namespace) else {
                return;
            };
            for name in ruby_call_argument_names(node, source) {
                state
                    .hidden_singleton_methods
                    .insert(ruby_singleton_method_key(&owner, &name));
                hide_ruby_singleton_method(signatures, &owner, &name);
            }
        }
        _ => {}
    }
}

fn extract_ruby_attribute_signatures(
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
    if !matches!(
        method.as_str(),
        "attr_reader" | "attr_writer" | "attr_accessor"
    ) {
        return Vec::new();
    }

    ruby_call_argument_names(node, source)
        .into_iter()
        .map(|name| {
            if namespace.is_empty() {
                format!("{method} {name}")
            } else {
                format!("{method} {}#{name}", namespace.join("::"))
            }
        })
        .collect()
}

fn ruby_call_method_name(node: Node<'_>, source: &str) -> Option<String> {
    node.child_by_field_name("method")
        .and_then(|value| node_text(value, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
}

fn ruby_method_name(node: Node<'_>, source: &str) -> Option<String> {
    node.child_by_field_name("name")
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
        "simple_symbol" | "delimited_symbol" | "bare_symbol" | "constant" | "string_content"
        | "self" => {
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

fn ruby_owner(namespace: &[String]) -> Option<String> {
    (!namespace.is_empty()).then(|| namespace.join("::"))
}

fn resolve_ruby_singleton_owner(
    singleton_receiver: Option<&str>,
    namespace: &[String],
) -> Option<String> {
    match singleton_receiver {
        Some("self") => ruby_owner(namespace),
        Some(owner) => Some(owner.to_string()),
        None => ruby_owner(namespace),
    }
}

fn ruby_singleton_method_key(owner: &str, name: &str) -> String {
    format!("{owner}.{name}")
}

fn ruby_module_function_key(owner: &str, name: &str) -> String {
    format!("{owner}#{name}")
}

fn ruby_module_function_applies(state: &RubyExportState, owner: &str, name: &str) -> bool {
    state.module_function_all_owners.contains(owner)
        || state
            .module_function_methods
            .contains(&ruby_module_function_key(owner, name))
}

fn ruby_call_has_self_argument(node: Node<'_>, source: &str) -> bool {
    ruby_call_argument_names(node, source)
        .into_iter()
        .any(|name| name == "self")
}

fn promote_ruby_module_function(
    signatures: &mut Vec<String>,
    state: &RubyExportState,
    owner: &str,
    name: &str,
) {
    let mut promoted = Vec::new();
    signatures.retain(|signature| {
        if !matches_ruby_instance_method_signature(signature, owner, name) {
            return true;
        }

        if state
            .hidden_singleton_methods
            .contains(&ruby_singleton_method_key(owner, name))
        {
            return false;
        }

        if let Some(singleton_signature) = instance_export_to_singleton_export(signature, owner) {
            promoted.push(singleton_signature);
        }
        false
    });

    for signature in promoted {
        push_unique(signatures, signature);
    }
}

fn export_ruby_instance_methods_as_singleton(
    signatures: &mut Vec<String>,
    state: &RubyExportState,
    owner: &str,
) {
    let mut promoted = Vec::new();
    for signature in signatures.iter() {
        let Some((sig_owner, method_name)) = ruby_instance_signature_metadata(signature) else {
            continue;
        };
        if sig_owner != owner
            || state
                .hidden_singleton_methods
                .contains(&ruby_singleton_method_key(&sig_owner, &method_name))
        {
            continue;
        }
        if let Some(singleton_signature) = instance_export_to_singleton_export(signature, owner) {
            promoted.push(singleton_signature);
        }
    }

    for signature in promoted {
        push_unique(signatures, signature);
    }
}

fn hide_ruby_singleton_method(signatures: &mut Vec<String>, owner: &str, name: &str) {
    signatures.retain(|signature| !matches_ruby_singleton_method_signature(signature, owner, name));
}

fn matches_ruby_instance_method_signature(signature: &str, owner: &str, name: &str) -> bool {
    let Some((sig_owner, sig_name)) = ruby_instance_signature_metadata(signature) else {
        return false;
    };
    sig_owner == owner && sig_name == name
}

fn matches_ruby_singleton_method_signature(signature: &str, owner: &str, name: &str) -> bool {
    let Some((sig_owner, sig_name)) = ruby_singleton_signature_metadata(signature) else {
        return false;
    };
    sig_owner == owner && sig_name == name
}

fn ruby_instance_signature_metadata(signature: &str) -> Option<(String, String)> {
    let rest = signature.strip_prefix("ruby:def ")?;
    let (owner, remainder) = rest.split_once('#')?;
    let method_name = remainder.split('(').next()?.trim();
    if owner.is_empty() || method_name.is_empty() {
        return None;
    }
    Some((owner.to_string(), method_name.to_string()))
}

fn ruby_singleton_signature_metadata(signature: &str) -> Option<(String, String)> {
    let rest = signature.strip_prefix("ruby:def ")?;
    let (owner, remainder) = rest.rsplit_once('.')?;
    let method_name = remainder.split('(').next()?.trim();
    if owner.is_empty() || method_name.is_empty() {
        return None;
    }
    Some((owner.to_string(), method_name.to_string()))
}

fn instance_export_to_singleton_export(signature: &str, owner: &str) -> Option<String> {
    let prefix = format!("ruby:def {owner}#");
    let remainder = signature.strip_prefix(&prefix)?;
    Some(format!("ruby:def {owner}.{remainder}"))
}

fn transform_ruby_instance_signature_to_singleton(signature: &str, owner: &str) -> Option<String> {
    let prefix = format!("def {owner}#");
    let remainder = signature.strip_prefix(&prefix)?;
    Some(normalize_tree_sitter_signature(&format!(
        "def {owner}.{remainder}"
    )))
}

fn push_unique(signatures: &mut Vec<String>, signature: String) {
    if !signatures.iter().any(|existing| existing == &signature) {
        signatures.push(signature);
    }
}

fn qualify_ruby_instance_signature(signature: &str, owner: &str) -> String {
    let Some(rest) = signature.strip_prefix("def ") else {
        return signature.to_string();
    };

    normalize_tree_sitter_signature(&format!("def {owner}#{rest}"))
}

fn qualify_ruby_receiver_signature(signature: &str, receiver: &str) -> String {
    let Some(rest) = signature.strip_prefix("def ") else {
        return signature.to_string();
    };

    normalize_tree_sitter_signature(&format!("def {receiver}.{}", rest.trim_start()))
}

fn qualify_ruby_singleton_signature(signature: &str, namespace: &[String]) -> String {
    if namespace.is_empty() {
        return signature.to_string();
    }

    let owner = namespace.join("::");
    if let Some(rest) = signature.strip_prefix("def self.") {
        return normalize_tree_sitter_signature(&format!("def {owner}.{rest}"));
    }

    signature.to_string()
}
