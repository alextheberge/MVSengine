// SPDX-License-Identifier: AGPL-3.0-only
use std::{collections::BTreeMap, fs, path::Path};

use serde_json::Value;
use tree_sitter::{Node, Parser};

use super::super::language::SourceLanguage;
use super::super::{
    extract_tree_sitter_prefix_signature, named_children, node_text, node_text_range,
    normalize_export_statement_signature, normalize_signature,
};
use crate::mvs::manifest::TsExportFollowing;

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub(crate) struct TsModuleIndex {
    exports_by_module: BTreeMap<String, BTreeMap<String, String>>,
    exact_workspace_specifiers: BTreeMap<String, Vec<String>>,
    wildcard_workspace_specifiers: Vec<TsWorkspaceAlias>,
    base_url: Option<String>,
    export_following: TsExportFollowing,
}

pub(crate) struct TsModuleSource<'a> {
    pub rel_path: &'a str,
    pub source: &'a str,
    pub language: SourceLanguage,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
struct TsWorkspaceConfig {
    exact_workspace_specifiers: BTreeMap<String, Vec<String>>,
    wildcard_workspace_specifiers: Vec<TsWorkspaceAlias>,
    base_url: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct TsWorkspaceAlias {
    specifier_prefix: String,
    specifier_suffix: String,
    target_prefix: String,
    target_suffix: String,
}

const TS_EXPORT_CONDITION_PRIORITY: &[&str] = &[
    "types", "import", "module", "browser", "node", "default", "require",
];

impl TsModuleIndex {
    fn with_workspace_config(
        workspace_config: &TsWorkspaceConfig,
        export_following: TsExportFollowing,
    ) -> Self {
        Self {
            exports_by_module: BTreeMap::new(),
            exact_workspace_specifiers: workspace_config.exact_workspace_specifiers.clone(),
            wildcard_workspace_specifiers: workspace_config.wildcard_workspace_specifiers.clone(),
            base_url: workspace_config.base_url.clone(),
            export_following,
        }
    }
}

pub(super) fn build_module_index(
    files: &[TsModuleSource<'_>],
    export_following: TsExportFollowing,
    root: &Path,
) -> TsModuleIndex {
    if export_following == TsExportFollowing::Off {
        return TsModuleIndex::default();
    }

    let workspace_config = load_ts_workspace_config(root, export_following);
    let mut index = TsModuleIndex::with_workspace_config(&workspace_config, export_following);

    for _ in 0..6 {
        let mut next = TsModuleIndex::with_workspace_config(&workspace_config, export_following);

        for file in files {
            let Some(grammar) = file.language.tree_sitter_language() else {
                continue;
            };
            let mut parser = Parser::new();
            if parser.set_language(&grammar).is_err() {
                continue;
            }
            let Some(tree) = parser.parse(file.source, None) else {
                continue;
            };

            let signatures = extract(tree.root_node(), file.source, file.rel_path, Some(&index));
            let exports = extract_export_map(&signatures);
            if exports.is_empty() {
                continue;
            }

            for module_name in ts_module_name_candidates(file.rel_path) {
                next.exports_by_module
                    .entry(module_name)
                    .or_default()
                    .extend(exports.clone());
            }
        }

        if next == index {
            break;
        }
        index = next;
    }

    index
}

fn load_ts_workspace_config(root: &Path, export_following: TsExportFollowing) -> TsWorkspaceConfig {
    if export_following != TsExportFollowing::WorkspaceOnly {
        return TsWorkspaceConfig::default();
    }

    let mut workspace_config = TsWorkspaceConfig::default();

    load_tsconfig_workspace_config(root, &mut workspace_config);
    load_package_json_workspace_config(root, &mut workspace_config);

    workspace_config
}

fn load_tsconfig_workspace_config(root: &Path, workspace_config: &mut TsWorkspaceConfig) {
    let config_path = ["tsconfig.json", "jsconfig.json"]
        .into_iter()
        .map(|candidate| root.join(candidate))
        .find(|candidate| candidate.exists());
    let Some(config_path) = config_path else {
        return;
    };
    let Ok(raw) = fs::read_to_string(config_path) else {
        return;
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
        return;
    };
    let Some(compiler_options) = parsed.get("compilerOptions").and_then(Value::as_object) else {
        return;
    };

    let base_url = compiler_options
        .get("baseUrl")
        .and_then(Value::as_str)
        .map(normalize_workspace_target)
        .filter(|value| !value.is_empty());

    if workspace_config.base_url.is_none() {
        workspace_config.base_url = base_url.clone();
    }

    let Some(paths) = compiler_options.get("paths").and_then(Value::as_object) else {
        return;
    };

    for (specifier, targets) in paths {
        let Some(target_values) = targets.as_array() else {
            continue;
        };
        for target in target_values.iter().filter_map(Value::as_str) {
            let normalized_target = normalize_tsconfig_path_target(target, base_url.as_deref());
            register_workspace_specifier(workspace_config, specifier, &normalized_target);
        }
    }
}

fn load_package_json_workspace_config(root: &Path, workspace_config: &mut TsWorkspaceConfig) {
    let package_path = root.join("package.json");
    let Ok(raw) = fs::read_to_string(package_path) else {
        return;
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
        return;
    };
    let Some(package_name) = parsed.get("name").and_then(Value::as_str) else {
        return;
    };
    let Some(exports) = parsed.get("exports") else {
        return;
    };

    collect_package_export_rules(package_name, None, exports, workspace_config);
}

fn collect_package_export_rules(
    package_name: &str,
    export_key: Option<&str>,
    value: &Value,
    workspace_config: &mut TsWorkspaceConfig,
) {
    match value {
        Value::String(target) => {
            let Some(specifier) =
                export_key.and_then(|key| package_export_specifier(package_name, key))
            else {
                return;
            };
            register_workspace_specifier(
                workspace_config,
                &specifier,
                &normalize_workspace_target(target),
            );
        }
        Value::Array(values) => {
            for value in values {
                collect_package_export_rules(package_name, export_key, value, workspace_config);
            }
        }
        Value::Object(map) => {
            let subpath_keys: Vec<&str> = map
                .keys()
                .map(String::as_str)
                .filter(|key| *key == "." || key.starts_with("./"))
                .collect();

            if export_key.is_none() && !subpath_keys.is_empty() {
                for nested_key in &subpath_keys {
                    let Some(nested_value) = map.get(*nested_key) else {
                        continue;
                    };
                    collect_package_export_rules(
                        package_name,
                        Some(nested_key),
                        nested_value,
                        workspace_config,
                    );
                }
                if subpath_keys.len() == map.len() {
                    return;
                }
            }

            collect_package_condition_rules(
                package_name,
                export_key.or(Some(".")),
                map,
                &subpath_keys,
                workspace_config,
            );
        }
        _ => {}
    }
}

fn collect_package_condition_rules(
    package_name: &str,
    export_key: Option<&str>,
    map: &serde_json::Map<String, Value>,
    subpath_keys: &[&str],
    workspace_config: &mut TsWorkspaceConfig,
) {
    for condition in TS_EXPORT_CONDITION_PRIORITY {
        let Some(value) = map.get(*condition) else {
            continue;
        };
        collect_package_export_rules(package_name, export_key, value, workspace_config);
    }

    let mut remaining_keys: Vec<&str> = map
        .keys()
        .map(String::as_str)
        .filter(|key| !subpath_keys.contains(key) && !TS_EXPORT_CONDITION_PRIORITY.contains(key))
        .collect();
    remaining_keys.sort();

    for key in remaining_keys {
        let Some(value) = map.get(key) else {
            continue;
        };
        collect_package_export_rules(package_name, export_key, value, workspace_config);
    }
}

fn package_export_specifier(package_name: &str, export_key: &str) -> Option<String> {
    if export_key == "." {
        return Some(package_name.to_string());
    }

    export_key
        .strip_prefix("./")
        .map(|path| format!("{package_name}/{}", path.trim_start_matches('/')))
}

fn register_workspace_specifier(
    workspace_config: &mut TsWorkspaceConfig,
    specifier: &str,
    target: &str,
) {
    let specifier = specifier.trim();
    let target = target.trim();
    if specifier.is_empty() || target.is_empty() {
        return;
    }

    let specifier_wildcards = specifier.matches('*').count();
    let target_wildcards = target.matches('*').count();
    if specifier_wildcards == 1 && target_wildcards == 1 {
        let Some((specifier_prefix, specifier_suffix)) = specifier.split_once('*') else {
            return;
        };
        let Some((target_prefix, target_suffix)) = target.split_once('*') else {
            return;
        };
        let alias = TsWorkspaceAlias {
            specifier_prefix: specifier_prefix.to_string(),
            specifier_suffix: specifier_suffix.to_string(),
            target_prefix: target_prefix.to_string(),
            target_suffix: target_suffix.to_string(),
        };
        if !workspace_config
            .wildcard_workspace_specifiers
            .contains(&alias)
        {
            workspace_config.wildcard_workspace_specifiers.push(alias);
        }
        return;
    }
    if specifier_wildcards == 0 && target_wildcards == 0 {
        let targets = workspace_config
            .exact_workspace_specifiers
            .entry(specifier.to_string())
            .or_default();
        if !targets.iter().any(|candidate| candidate == target) {
            targets.push(target.to_string());
        }
    }
}

fn normalize_tsconfig_path_target(target: &str, base_url: Option<&str>) -> String {
    let target = normalize_workspace_target(target);
    match base_url {
        Some(base_url) if !base_url.is_empty() => {
            normalize_workspace_target(&format!("{base_url}/{target}"))
        }
        _ => target,
    }
}

fn normalize_workspace_target(target: &str) -> String {
    target
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_matches('/')
        .to_string()
}

pub(super) fn extract(
    root: Node<'_>,
    source: &str,
    rel_path: &str,
    module_index: Option<&TsModuleIndex>,
) -> Vec<String> {
    let mut signatures = Vec::new();
    for child in named_children(root) {
        if child.kind() != "export_statement" {
            continue;
        }

        signatures.extend(extract_export_statement(
            child,
            source,
            rel_path,
            module_index,
        ));
    }

    signatures
}

fn extract_export_statement(
    node: Node<'_>,
    source: &str,
    rel_path: &str,
    module_index: Option<&TsModuleIndex>,
) -> Vec<String> {
    let is_default_export = node_text(node, source)
        .map(|text| text.trim_start().starts_with("export default"))
        .unwrap_or(false);

    if let Some(declaration) = node.child_by_field_name("declaration") {
        let signatures = extract_export_declaration(declaration, source);
        if !signatures.is_empty() {
            return signatures
                .into_iter()
                .map(|signature| {
                    if is_default_export {
                        format!("ts/js:export default {signature}")
                    } else {
                        format!("ts/js:{signature}")
                    }
                })
                .collect();
        }
    }

    if let Some(value) = node.child_by_field_name("value") {
        if let Some(signature) = extract_default_export_value(value, source) {
            return vec![format!("ts/js:export default {signature}")];
        }
    }

    let statement = node_text(node, source)
        .map(normalize_export_statement_signature)
        .filter(|signature| !signature.is_empty());
    let Some(statement) = statement else {
        return Vec::new();
    };

    if let Some(index) = module_index {
        let followed = follow_reexport_signatures(&statement, rel_path, index);
        if !followed.is_empty() {
            return followed;
        }
    }

    vec![format!("ts/js:{statement}")]
}

fn extract_export_declaration(node: Node<'_>, source: &str) -> Vec<String> {
    match node.kind() {
        "function_declaration" | "generator_function_declaration" => {
            extract_function_signature(node, source)
                .into_iter()
                .collect()
        }
        "class_declaration" => extract_tree_sitter_prefix_signature(node, source, "body")
            .into_iter()
            .collect(),
        "interface_declaration" => extract_tree_sitter_prefix_signature(node, source, "body")
            .into_iter()
            .collect(),
        "enum_declaration" => extract_tree_sitter_prefix_signature(node, source, "body")
            .into_iter()
            .collect(),
        "type_alias_declaration" => node_text(node, source)
            .map(normalize_export_statement_signature)
            .filter(|signature| !signature.is_empty())
            .into_iter()
            .collect(),
        "lexical_declaration" => extract_variable_signatures(node, source),
        "variable_declaration" => extract_variable_signatures(node, source),
        _ => Vec::new(),
    }
}

fn extract_default_export_value(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "function" | "function_expression" | "generator_function" => {
            extract_function_signature(node, source)
        }
        "class" => extract_tree_sitter_prefix_signature(node, source, "body"),
        _ => None,
    }
}

fn extract_function_signature(node: Node<'_>, source: &str) -> Option<String> {
    let body = node.child_by_field_name("body")?;
    node_text_range(source, node.start_byte(), body.start_byte())
        .map(normalize_export_statement_signature)
        .filter(|signature| !signature.is_empty())
}

fn extract_variable_signatures(node: Node<'_>, source: &str) -> Vec<String> {
    let kind = node
        .child_by_field_name("kind")
        .and_then(|child| node_text(child, source))
        .map(str::trim)
        .unwrap_or("var");

    let mut signatures = Vec::new();
    for index in 0..node.named_child_count() {
        let Some(child) = node.named_child(index) else {
            continue;
        };
        if child.kind() != "variable_declarator" {
            continue;
        }

        let Some(name) = child.child_by_field_name("name") else {
            continue;
        };
        if name.kind() != "identifier" {
            continue;
        }

        let Some(name_text) = node_text(name, source).map(str::trim) else {
            continue;
        };
        if name_text.is_empty() {
            continue;
        }

        let mut signature = format!("{kind} {name_text}");
        if let Some(type_annotation) = child
            .child_by_field_name("type")
            .and_then(|annotation| node_text(annotation, source))
            .map(str::trim)
        {
            if !type_annotation.is_empty() {
                signature.push_str(type_annotation);
            }
        }

        let normalized = normalize_signature(&signature);
        if !normalized.is_empty() {
            signatures.push(normalized);
        }
    }

    signatures
}

fn follow_reexport_signatures(
    statement: &str,
    rel_path: &str,
    module_index: &TsModuleIndex,
) -> Vec<String> {
    let Some((clause, source_specifier)) = parse_reexport_statement(statement) else {
        return Vec::new();
    };
    let Some(target_exports) = resolve_ts_module_exports(module_index, rel_path, &source_specifier)
    else {
        return Vec::new();
    };

    let normalized_clause = clause.trim();
    if normalized_clause == "*" || normalized_clause == "type *" {
        return target_exports
            .iter()
            .filter(|(name, _)| name.as_str() != "default")
            .map(|(_, signature)| signature.clone())
            .collect();
    }

    if normalized_clause.starts_with("* as ") {
        return Vec::new();
    }

    let Some(inner) = normalized_clause
        .strip_prefix("type ")
        .unwrap_or(normalized_clause)
        .trim()
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
    else {
        return Vec::new();
    };

    let mut signatures = Vec::new();
    for item in inner
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let item = item.strip_prefix("type ").unwrap_or(item).trim();
        let (import_name, export_name) = if let Some((left, right)) = item.split_once(" as ") {
            (left.trim(), right.trim())
        } else {
            (item, item)
        };

        let Some(target_signature) = target_exports.get(import_name) else {
            continue;
        };
        if let Some(signature) = rename_ts_export_signature(target_signature, export_name) {
            signatures.push(signature);
        }
    }

    signatures
}

fn parse_reexport_statement(statement: &str) -> Option<(String, String)> {
    let statement = statement.strip_prefix("export ")?;
    let (clause, source_specifier) = statement.rsplit_once(" from ")?;
    let source_specifier = source_specifier.trim();
    let source_specifier = source_specifier
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            source_specifier
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })?;
    Some((clause.trim().to_string(), source_specifier.to_string()))
}

fn resolve_ts_module_exports<'a>(
    module_index: &'a TsModuleIndex,
    rel_path: &str,
    source_specifier: &str,
) -> Option<&'a BTreeMap<String, String>> {
    let resolved = if source_specifier.starts_with("./") || source_specifier.starts_with("../") {
        resolve_relative_ts_module_specifier(rel_path, source_specifier)?
    } else {
        resolve_workspace_ts_module_specifier(module_index, source_specifier)?
    };
    module_index.exports_by_module.get(&resolved).or_else(|| {
        strip_supported_ts_extension(&resolved)
            .and_then(|key| module_index.exports_by_module.get(key))
    })
}

fn resolve_relative_ts_module_specifier(rel_path: &str, source_specifier: &str) -> Option<String> {
    let mut parts: Vec<&str> = rel_path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if !parts.is_empty() {
        parts.pop();
    }

    for part in source_specifier.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }

    (!parts.is_empty()).then(|| parts.join("/"))
}

fn resolve_workspace_ts_module_specifier(
    module_index: &TsModuleIndex,
    source_specifier: &str,
) -> Option<String> {
    if module_index.export_following != TsExportFollowing::WorkspaceOnly {
        return None;
    }

    if let Some(targets) = module_index
        .exact_workspace_specifiers
        .get(source_specifier)
    {
        return resolve_workspace_target_candidates(
            module_index,
            targets.iter().map(String::as_str),
        );
    }
    if let Some(stripped) = strip_supported_ts_extension(source_specifier) {
        if let Some(targets) = module_index.exact_workspace_specifiers.get(stripped) {
            return resolve_workspace_target_candidates(
                module_index,
                targets.iter().map(String::as_str),
            );
        }
    }

    let mut wildcard_targets = Vec::new();
    for alias in &module_index.wildcard_workspace_specifiers {
        if let Some(target) = resolve_workspace_alias(alias, source_specifier) {
            wildcard_targets.push(target);
        }
    }
    if let Some(stripped) = strip_supported_ts_extension(source_specifier) {
        for alias in &module_index.wildcard_workspace_specifiers {
            if let Some(target) = resolve_workspace_alias(alias, stripped) {
                wildcard_targets.push(target);
            }
        }
    }
    if !wildcard_targets.is_empty() {
        return resolve_workspace_target_candidates(
            module_index,
            wildcard_targets.iter().map(String::as_str),
        );
    }

    module_index.base_url.as_ref().and_then(|base_url| {
        resolve_workspace_target_candidates(
            module_index,
            [normalize_workspace_target(&format!(
                "{base_url}/{source_specifier}"
            ))]
            .iter()
            .map(String::as_str),
        )
    })
}

fn resolve_workspace_alias(alias: &TsWorkspaceAlias, source_specifier: &str) -> Option<String> {
    if !source_specifier.starts_with(&alias.specifier_prefix)
        || !source_specifier.ends_with(&alias.specifier_suffix)
    {
        return None;
    }
    let wildcard_value = &source_specifier
        [alias.specifier_prefix.len()..source_specifier.len() - alias.specifier_suffix.len()];
    Some(normalize_workspace_target(&format!(
        "{}{}{}",
        alias.target_prefix, wildcard_value, alias.target_suffix
    )))
}

fn resolve_workspace_target_candidates<'a>(
    module_index: &TsModuleIndex,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let mut fallback = None;
    for candidate in candidates {
        if fallback.is_none() {
            fallback = Some(candidate.to_string());
        }
        if workspace_target_exists(module_index, candidate) {
            return Some(candidate.to_string());
        }
    }
    fallback
}

fn workspace_target_exists(module_index: &TsModuleIndex, target: &str) -> bool {
    module_index.exports_by_module.contains_key(target)
        || strip_supported_ts_extension(target)
            .map(|stripped| module_index.exports_by_module.contains_key(stripped))
            .unwrap_or(false)
}

fn strip_supported_ts_extension(path: &str) -> Option<&str> {
    ["ts", "tsx", "js", "jsx"]
        .into_iter()
        .find_map(|extension| path.strip_suffix(&format!(".{extension}")))
}

fn rename_ts_export_signature(signature: &str, export_name: &str) -> Option<String> {
    let raw = signature.strip_prefix("ts/js:")?;
    if export_name == "default" {
        let normalized = raw.strip_prefix("export default ").unwrap_or(raw);
        return Some(format!("ts/js:export default {normalized}"));
    }

    let raw = raw.strip_prefix("export default ").unwrap_or(raw);
    let renamed = if let Some(rest) = raw.strip_prefix("async function ") {
        format!("async function {export_name}{}", split_ts_name_tail(rest))
    } else if let Some(rest) = raw.strip_prefix("function ") {
        format!("function {export_name}{}", split_ts_name_tail(rest))
    } else if let Some(rest) = raw.strip_prefix("class ") {
        format!("class {export_name}{}", split_ts_name_tail(rest))
    } else if let Some(rest) = raw.strip_prefix("interface ") {
        format!("interface {export_name}{}", split_ts_name_tail(rest))
    } else if let Some(rest) = raw.strip_prefix("enum ") {
        format!("enum {export_name}{}", split_ts_name_tail(rest))
    } else if let Some(rest) = raw.strip_prefix("type ") {
        format!("type {export_name}{}", split_ts_name_tail(rest))
    } else if let Some(rest) = raw.strip_prefix("const ") {
        format!("const {export_name}{}", split_ts_name_tail(rest))
    } else if let Some(rest) = raw.strip_prefix("let ") {
        format!("let {export_name}{}", split_ts_name_tail(rest))
    } else if let Some(rest) = raw.strip_prefix("var ") {
        format!("var {export_name}{}", split_ts_name_tail(rest))
    } else {
        return None;
    };

    Some(format!("ts/js:{renamed}"))
}

fn split_ts_name_tail(rest: &str) -> &str {
    for (index, character) in rest.char_indices() {
        if matches!(character, '(' | '<' | ':' | '=' | ' ' | '{') {
            return &rest[index..];
        }
    }

    ""
}

fn extract_export_map(signatures: &[String]) -> BTreeMap<String, String> {
    let mut exports = BTreeMap::new();

    for signature in signatures {
        for export_name in export_names_from_signature(signature) {
            exports
                .entry(export_name)
                .or_insert_with(|| signature.clone());
        }
    }

    exports
}

fn export_names_from_signature(signature: &str) -> Vec<String> {
    if signature.starts_with("ts/js:export default ") {
        return vec!["default".to_string()];
    }

    if let Some(rest) = signature.strip_prefix("ts/js:export * as ") {
        let name = rest
            .split(" from ")
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        return name
            .map(|value| vec![value.to_string()])
            .unwrap_or_default();
    }

    if let Some(rest) = signature.strip_prefix("ts/js:export ") {
        if let Some(inner) = rest
            .strip_prefix("type ")
            .unwrap_or(rest)
            .trim()
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
        {
            return inner
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .filter_map(|value| {
                    let value = value.strip_prefix("type ").unwrap_or(value).trim();
                    value
                        .split(" as ")
                        .nth(1)
                        .or(Some(value))
                        .map(str::trim)
                        .filter(|candidate| !candidate.is_empty())
                        .map(ToString::to_string)
                })
                .collect();
        }
    }

    extract_named_signature_identifier(signature)
        .into_iter()
        .collect()
}

fn extract_named_signature_identifier(signature: &str) -> Option<String> {
    let raw = signature.strip_prefix("ts/js:")?;
    let raw = raw.strip_prefix("export default ").unwrap_or(raw);

    for prefix in [
        "async function ",
        "function ",
        "class ",
        "interface ",
        "enum ",
        "type ",
        "const ",
        "let ",
        "var ",
    ] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            let name = rest
                .split(['(', '<', ':', '=', ' ', '{'])
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            return Some(name.to_string());
        }
    }

    None
}

fn ts_module_name_candidates(rel_path: &str) -> Vec<String> {
    let normalized = rel_path.replace('\\', "/");
    let mut candidates = vec![normalized.clone()];
    if let Some(stripped) = strip_supported_ts_extension(&normalized) {
        candidates.push(stripped.to_string());
        if let Some(directory) = stripped.strip_suffix("/index") {
            if !directory.is_empty() {
                candidates.push(directory.to_string());
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates
}
