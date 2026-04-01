// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::super::{
    extract_tree_sitter_prefix_signature, is_exported_tree_sitter_name, named_children,
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
            _ => {}
        }
    }

    signatures
}
