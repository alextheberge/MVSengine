// SPDX-License-Identifier: AGPL-3.0-only
mod adapters;
mod language;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use quote::ToTokens;
use regex::Regex;
use serde::{Deserialize, Serialize};
use syn::{FnArg, ImplItem, Item, ReturnType, Signature, TraitItem, Type, UseTree, Visibility};
use tree_sitter::{Node, Parser as TreeSitterParser};
use walkdir::{DirEntry, WalkDir};

use crate::mvs::manifest::{GoExportFollowing, RustExportFollowing, ScanPolicy};

use self::language::{LexStrategy, SourceLanguage};

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct TagOccurrence {
    pub name: String,
    pub file: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct ApiSignature {
    pub file: String,
    pub signature: String,
}

#[derive(Debug, Clone, Default)]
pub struct CrawlReport {
    pub feature_tags: BTreeSet<String>,
    pub protocol_tags: BTreeSet<String>,
    pub feature_occurrences: Vec<TagOccurrence>,
    pub protocol_occurrences: Vec<TagOccurrence>,
    pub public_api: Vec<ApiSignature>,
    pub public_api_boundary_decisions: Vec<PublicApiBoundaryDecision>,
    pub excluded_paths: Vec<ExcludedPathDecision>,
}

#[derive(Debug, Clone, Serialize, Eq, PartialEq, Ord, PartialOrd)]
pub struct PublicApiBoundaryDecision {
    pub file: String,
    pub signature: String,
    pub included: bool,
    pub file_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_rule: Option<String>,
    pub item_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_rule: Option<String>,
}

#[derive(Debug, Clone, Serialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum ExcludedPathKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Serialize, Eq, PartialEq, Ord, PartialOrd)]
pub struct ExcludedPathDecision {
    pub path: String,
    pub kind: ExcludedPathKind,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
}

#[derive(Debug, Clone)]
struct LexedSource {
    comments: Vec<String>,
    masked_code: String,
}

struct SourceFileInput {
    rel: String,
    language: SourceLanguage,
    source: String,
    lexed: LexedSource,
}

#[derive(Debug, Clone)]
struct PublicApiFileDecision {
    included: bool,
    reason: &'static str,
    rule: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct RustModuleIndex {
    public_module_files_by_rel_path: BTreeMap<String, Vec<String>>,
    reexported_signatures_by_file: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default)]
struct RustWorkspaceCatalog {
    descriptors: Vec<RustCrateDescriptor>,
    crate_names: BTreeSet<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct RustCrateDescriptor {
    crate_name: String,
    source_root_rel: String,
    crate_root_rel: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct RustReexportedSymbol {
    symbol_path: String,
    signature: String,
    crate_alias: Option<String>,
}

struct RustReexportLookup<'a> {
    crate_alias: Option<&'a str>,
    rust_crate_names: &'a BTreeSet<String>,
    symbol_signatures_by_path: &'a BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct CargoManifestToml {
    package: Option<CargoPackageSection>,
    lib: Option<CargoLibSection>,
}

#[derive(Debug, Deserialize)]
struct CargoPackageSection {
    name: String,
}

#[derive(Debug, Deserialize)]
struct CargoLibSection {
    name: Option<String>,
    path: Option<String>,
}

impl RustModuleIndex {
    fn public_module_files_for<'a>(&'a self, rel_path: &str) -> &'a [String] {
        self.public_module_files_by_rel_path
            .get(rel_path)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn reexported_signatures_for<'a>(&'a self, rel_path: &str) -> &'a [String] {
        self.reexported_signatures_by_file
            .get(rel_path)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

impl RustWorkspaceCatalog {
    fn descriptor_for_file<'a>(&'a self, rel_path: &str) -> Option<&'a RustCrateDescriptor> {
        self.descriptors
            .iter()
            .filter(|descriptor| path_has_prefix(rel_path, &descriptor.source_root_rel))
            .max_by_key(|descriptor| descriptor.source_root_rel.len())
    }
}

struct ApiRegexPack {
    go_func: Regex,
    py_def: Regex,
    java_type: Regex,
    java_method: Regex,
    kt_decl: Regex,
    cs_type: Regex,
    cs_method: Regex,
    dart_library: Regex,
    dart_class: Regex,
    dart_mixin: Regex,
    dart_enum: Regex,
    dart_extension_type: Regex,
    dart_extension: Regex,
    dart_typedef_equals: Regex,
    dart_callable: Regex,
    dart_getter: Regex,
    dart_setter: Regex,
    dart_static_const: Regex,
    dart_field: Regex,
}

struct PublicApiExtractionContext<'a> {
    regex: &'a ApiRegexPack,
    scan_policy: &'a ScanPolicy,
    rust_module_index: &'a RustModuleIndex,
    ts_module_index: &'a adapters::TsModuleIndex,
    python_module_index: &'a adapters::PythonModuleIndex,
}

impl ApiRegexPack {
    fn new() -> Result<Self> {
        Ok(Self {
            go_func: Regex::new(
                r"^\s*func\s*(?P<recv>\([^)]*\)\s*)?(?P<name>[A-Z][A-Za-z0-9_]*)\s*(?P<sig>\([^)]*\)\s*[^\{]*)",
            )
            .context("failed to compile Go API regex")?,
            py_def: Regex::new(
                r"^\s*def\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?P<sig>\([^)]*\)\s*(?:->\s*[^:]+)?)\s*:",
            )
            .context("failed to compile Python API regex")?,
            java_type: Regex::new(
                r"^\s*public\s+(?:abstract\s+)?(?:final\s+)?(?:class|interface|enum)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
            )
            .context("failed to compile Java API regex (type)")?,
            java_method: Regex::new(
                r"^\s*public\s+(?:static\s+)?(?:final\s+)?[A-Za-z0-9_<>,\[\].?\s]+\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?P<sig>\([^)]*\))",
            )
            .context("failed to compile Java API regex (method)")?,
            kt_decl: Regex::new(
                r"^\s*(?:public\s+)?(?P<kind>class|interface|fun)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?P<rest>[^\{=]*)",
            )
            .context("failed to compile Kotlin API regex")?,
            cs_type: Regex::new(
                r"^\s*public\s+(?:sealed\s+)?(?:static\s+)?(?:class|interface|enum|struct)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
            )
            .context("failed to compile C# API regex (type)")?,
            cs_method: Regex::new(
                r"^\s*public\s+(?:static\s+)?[A-Za-z0-9_<>,\[\].?\s]+\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?P<sig>\([^)]*\))",
            )
            .context("failed to compile C# API regex (method)")?,
            dart_library: Regex::new(r"^\s*library\s+(?P<lib>[\w.$]+)\s*;")
                .context("failed to compile Dart library regex")?,
            dart_class: Regex::new(
                r"^\s*(?:abstract\s+|sealed\s+|base\s+|interface\s+|final\s+)*class\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
            )
            .context("failed to compile Dart class regex")?,
            dart_mixin: Regex::new(r"^\s*(?:base\s+)?mixin\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)")
                .context("failed to compile Dart mixin regex")?,
            dart_enum: Regex::new(r"^\s*enum\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)")
                .context("failed to compile Dart enum regex")?,
            dart_extension_type: Regex::new(
                r"^\s*extension\s+type\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
            )
            .context("failed to compile Dart extension type regex")?,
            dart_extension: Regex::new(
                r"^\s*extension\s+(?:(?P<named>[A-Za-z_][A-Za-z0-9_]*)\s+)?on\s+(?P<on>[^{]+)",
            )
            .context("failed to compile Dart extension regex")?,
            dart_typedef_equals: Regex::new(
                r"^\s*typedef\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=",
            )
            .context("failed to compile Dart typedef regex")?,
            dart_callable: Regex::new(
                r"^\s*(?:@[\w.$]+\s+)*(?:external\s+)?(?:static\s+)?(?:async\s+)?(?:covariant\s+)?(?P<ret>void|[A-Za-z_][\w<>,\s\?\[\].]*)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\((?P<args>[^)]*)\)\s*(?:async\s*)?(?:\{|=>.*;|;)\s*$",
            )
            .context("failed to compile Dart callable regex")?,
            dart_getter: Regex::new(
                r"^\s*(?:@[\w.$]+\s+)*(?:static\s+)?(?P<ret>void|[A-Za-z_][\w<>,\s\?\[\].]*)\s+get\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?:=>|\{)",
            )
            .context("failed to compile Dart getter regex")?,
            dart_setter: Regex::new(
                r"^\s*(?:static\s+)?void\s+set\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(",
            )
            .context("failed to compile Dart setter regex")?,
            dart_static_const: Regex::new(
                r"^\s*static\s+const\s+(?P<ty>[A-Za-z_][\w<>,\s\?\[\].]*)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=",
            )
            .context("failed to compile Dart static const regex")?,
            dart_field: Regex::new(
                r"^\s*(?:static\s+)?(?:late\s+)?(?:final\s+)?(?P<ty>[A-Za-z_][\w<>,\s\?\[\].]*)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*[=;]",
            )
            .context("failed to compile Dart field regex")?,
        })
    }
}

pub fn crawl_codebase(root: &Path, scan_policy: &ScanPolicy) -> Result<CrawlReport> {
    let feature_re = Regex::new(
        r#"@mvs-feature\s*(?:\(\s*["'](?P<name>[^"']+)["'](?:\s*,\s*(?P<meta>[^)]*))?\)|:\s*(?P<name2>[A-Za-z0-9._-]+))"#,
    )
    .context("failed to compile feature regex")?;
    let protocol_re = Regex::new(
        r#"@mvs-protocol\s*(?:\(\s*["'](?P<surface>[^"']+)["'](?:\s*,\s*(?P<meta>[^)]*))?\)|:\s*(?P<surface2>[A-Za-z0-9._-]+))"#,
    )
    .context("failed to compile protocol regex")?;
    let api_pack = ApiRegexPack::new()?;
    let mut source_files = Vec::new();
    let mut excluded_paths = Vec::new();

    let mut report = CrawlReport::default();

    for entry in WalkDir::new(root).into_iter().filter_entry(|entry| {
        if let Some(decision) = exclusion_decision(root, scan_policy, entry) {
            excluded_paths.push(decision);
            false
        } else {
            true
        }
    }) {
        let entry = entry.with_context(|| format!("failed walking {}", root.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let Some(language) = SourceLanguage::from_path(path) else {
            continue;
        };
        let rel = relative_display_path(root, path);

        let source = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let lexed = lex_source(&source, language);

        source_files.push(SourceFileInput {
            rel,
            language,
            source,
            lexed,
        });
    }

    let python_module_sources: Vec<_> = source_files
        .iter()
        .filter(|file| file.language == SourceLanguage::Python)
        .map(|file| adapters::PythonModuleSource {
            rel_path: &file.rel,
            source: &file.source,
        })
        .collect();
    let rust_module_index = build_rust_module_index(root, &source_files, scan_policy);
    let go_package_sources: Vec<_> = source_files
        .iter()
        .filter(|file| file.language == SourceLanguage::Go)
        .map(|file| adapters::GoPackageSource {
            rel_path: &file.rel,
            source: &file.source,
        })
        .collect();
    let ts_module_sources: Vec<_> = source_files
        .iter()
        .filter(|file| {
            matches!(
                file.language,
                SourceLanguage::TypeScript
                    | SourceLanguage::Tsx
                    | SourceLanguage::JavaScript
                    | SourceLanguage::Jsx
            )
        })
        .map(|file| adapters::TsModuleSource {
            rel_path: &file.rel,
            source: &file.source,
            language: file.language,
        })
        .collect();
    let go_package_index =
        adapters::build_go_package_index(&go_package_sources, scan_policy.go_export_following);
    let ts_module_index =
        adapters::build_ts_module_index(&ts_module_sources, scan_policy.ts_export_following, root);
    let python_module_index = adapters::build_python_module_index(
        &python_module_sources,
        scan_policy.python_export_following,
        &scan_policy.python_module_roots,
    );
    let effective_public_api_files = build_effective_public_api_files(
        &source_files,
        scan_policy,
        &go_package_index,
        &rust_module_index,
    );

    for file in source_files {
        for tag in extract_named_tags(&file.lexed.comments, &feature_re, "name", "name2") {
            report.feature_tags.insert(tag.clone());
            report.feature_occurrences.push(TagOccurrence {
                name: tag,
                file: file.rel.clone(),
            });
        }

        for tag in extract_named_tags(&file.lexed.comments, &protocol_re, "surface", "surface2") {
            report.protocol_tags.insert(tag.clone());
            report.protocol_occurrences.push(TagOccurrence {
                name: tag,
                file: file.rel.clone(),
            });
        }

        let signatures = extract_public_api(
            file.language,
            &file.rel,
            &file.source,
            &file.lexed.masked_code,
            PublicApiExtractionContext {
                regex: &api_pack,
                scan_policy,
                rust_module_index: &rust_module_index,
                ts_module_index: &ts_module_index,
                python_module_index: &python_module_index,
            },
        );
        let file_decision = public_api_file_decision(&effective_public_api_files, &file.rel);
        for signature in signatures {
            let item_decision = if file_decision.included {
                scan_policy.public_api_item_filter_decision(&file.rel, &signature)
            } else {
                crate::mvs::manifest::PublicApiItemFilterDecision {
                    included: false,
                    reason: "skipped_file_boundary",
                    rule: None,
                }
            };
            let included = file_decision.included && item_decision.included;
            report
                .public_api_boundary_decisions
                .push(PublicApiBoundaryDecision {
                    file: file.rel.clone(),
                    signature: signature.clone(),
                    included,
                    file_reason: file_decision.reason.to_string(),
                    file_rule: file_decision.rule.clone(),
                    item_reason: item_decision.reason.to_string(),
                    item_rule: item_decision.rule.clone(),
                });
            if included {
                report.public_api.push(ApiSignature {
                    file: file.rel.clone(),
                    signature,
                });
            }
        }
    }

    report.feature_occurrences.sort();
    report.protocol_occurrences.sort();
    report.public_api.sort();
    report.public_api.dedup();
    report.public_api_boundary_decisions.sort();
    report.public_api_boundary_decisions.dedup();
    report.excluded_paths = excluded_paths;
    report.excluded_paths.sort();
    report.excluded_paths.dedup();

    Ok(report)
}

fn build_effective_public_api_files(
    source_files: &[SourceFileInput],
    scan_policy: &ScanPolicy,
    go_package_index: &adapters::GoPackageIndex,
    rust_module_index: &RustModuleIndex,
) -> Option<BTreeMap<String, PublicApiFileDecision>> {
    if scan_policy.public_api_roots.is_empty() {
        return None;
    }

    let mut files = BTreeMap::new();
    for file in source_files {
        if !scan_policy.includes_public_api(&file.rel) {
            continue;
        }

        files.insert(
            file.rel.clone(),
            PublicApiFileDecision {
                included: true,
                reason: "public_api_root",
                rule: scan_policy.matching_public_api_root(&file.rel),
            },
        );
        if file.language == SourceLanguage::Go
            && scan_policy.go_export_following == GoExportFollowing::PackageOnly
        {
            for package_file in go_package_index.package_files_for(&file.rel) {
                files
                    .entry(package_file.clone())
                    .or_insert_with(|| PublicApiFileDecision {
                        included: true,
                        reason: "go_export_following",
                        rule: Some("package_only".to_string()),
                    });
            }
        }
        if file.language == SourceLanguage::Rust
            && scan_policy.rust_export_following == RustExportFollowing::PublicModules
        {
            for module_file in rust_module_index.public_module_files_for(&file.rel) {
                files
                    .entry(module_file.clone())
                    .or_insert_with(|| PublicApiFileDecision {
                        included: true,
                        reason: "rust_export_following",
                        rule: Some("public_modules".to_string()),
                    });
            }
        }
    }

    Some(files)
}

fn public_api_file_decision(
    effective_public_api_files: &Option<BTreeMap<String, PublicApiFileDecision>>,
    relative_path: &str,
) -> PublicApiFileDecision {
    match effective_public_api_files {
        Some(files) => files
            .get(relative_path)
            .cloned()
            .unwrap_or(PublicApiFileDecision {
                included: false,
                reason: "outside_public_api_roots",
                rule: None,
            }),
        None => PublicApiFileDecision {
            included: true,
            reason: "default_allow",
            rule: None,
        },
    }
}

fn build_rust_module_index(
    root: &Path,
    source_files: &[SourceFileInput],
    scan_policy: &ScanPolicy,
) -> RustModuleIndex {
    if scan_policy.rust_export_following == RustExportFollowing::Off {
        return RustModuleIndex::default();
    }

    let rust_files: Vec<&SourceFileInput> = source_files
        .iter()
        .filter(|file| file.language == SourceLanguage::Rust)
        .collect();
    let rust_workspace_catalog = build_rust_workspace_catalog(root, &rust_files, scan_policy);
    let known_rust_files: BTreeSet<String> =
        rust_files.iter().map(|file| file.rel.clone()).collect();
    let mut edges_by_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut symbol_signatures_by_path: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for file in &rust_files {
        let Ok(parsed) = syn::parse_file(&file.source) else {
            continue;
        };
        let crate_descriptor = rust_workspace_catalog.descriptor_for_file(&file.rel);
        let module_dir = rust_module_directory(Path::new(&file.rel), crate_descriptor);
        let module_path = rust_file_module_path(Path::new(&file.rel), crate_descriptor);
        let crate_alias = crate_descriptor
            .filter(|descriptor| {
                rust_workspace_catalog
                    .crate_names
                    .contains(&descriptor.crate_name)
            })
            .map(|descriptor| descriptor.crate_name.as_str());
        let mut edges = BTreeSet::new();
        collect_rust_public_module_edges(&parsed.items, &module_dir, &known_rust_files, &mut edges);
        collect_rust_symbol_signatures(
            &parsed.items,
            &module_path,
            crate_alias,
            &rust_workspace_catalog.crate_names,
            &mut symbol_signatures_by_path,
        );
        if !edges.is_empty() {
            edges_by_file.insert(file.rel.clone(), edges.into_iter().collect());
        }
    }

    for _ in 0..rust_files.len().saturating_mul(2).max(1) {
        let mut pending = BTreeSet::new();
        for file in &rust_files {
            let Ok(parsed) = syn::parse_file(&file.source) else {
                continue;
            };
            let crate_alias = rust_workspace_catalog
                .descriptor_for_file(&file.rel)
                .filter(|descriptor| {
                    rust_workspace_catalog
                        .crate_names
                        .contains(&descriptor.crate_name)
                })
                .map(|descriptor| descriptor.crate_name.as_str());
            let current_module = rust_file_module_path(
                Path::new(&file.rel),
                rust_workspace_catalog.descriptor_for_file(&file.rel),
            );
            let lookup = RustReexportLookup {
                crate_alias,
                rust_crate_names: &rust_workspace_catalog.crate_names,
                symbol_signatures_by_path: &symbol_signatures_by_path,
            };
            collect_rust_reexported_symbols(
                &parsed.items,
                &current_module,
                true,
                &lookup,
                &mut pending,
            );
        }

        let mut changed = false;
        for symbol in pending {
            changed |= insert_rust_symbol_signature_variant(
                &mut symbol_signatures_by_path,
                &symbol.symbol_path,
                symbol.signature,
                symbol.crate_alias.as_deref(),
            );
        }
        if !changed {
            break;
        }
    }

    let mut index = RustModuleIndex::default();
    for rel_path in known_rust_files {
        let mut visited = BTreeSet::new();
        let mut stack = edges_by_file.get(&rel_path).cloned().unwrap_or_default();
        while let Some(next) = stack.pop() {
            if !visited.insert(next.clone()) {
                continue;
            }
            if let Some(children) = edges_by_file.get(&next) {
                stack.extend(children.iter().cloned());
            }
        }
        if !visited.is_empty() {
            index
                .public_module_files_by_rel_path
                .insert(rel_path, visited.into_iter().collect());
        }
    }

    for file in &rust_files {
        let Ok(parsed) = syn::parse_file(&file.source) else {
            continue;
        };
        let current_module = rust_file_module_path(
            Path::new(&file.rel),
            rust_workspace_catalog.descriptor_for_file(&file.rel),
        );
        let mut signatures = BTreeSet::new();
        collect_rust_reexported_signatures(
            &parsed.items,
            &current_module,
            true,
            &rust_workspace_catalog.crate_names,
            &symbol_signatures_by_path,
            &mut signatures,
        );
        if !signatures.is_empty() {
            index
                .reexported_signatures_by_file
                .insert(file.rel.clone(), signatures.into_iter().collect());
        }
    }

    index
}

fn build_rust_workspace_catalog(
    root: &Path,
    rust_files: &[&SourceFileInput],
    scan_policy: &ScanPolicy,
) -> RustWorkspaceCatalog {
    let mut manifests: BTreeMap<String, bool> = BTreeMap::new();
    for file in rust_files {
        if let Some(manifest_rel) = find_nearest_cargo_manifest_rel(root, &root.join(&file.rel)) {
            manifests.entry(manifest_rel).or_insert(false);
        }
    }

    for member in &scan_policy.rust_workspace_members {
        if let Some(manifest_rel) = resolve_rust_workspace_member_manifest_rel(root, member) {
            manifests
                .entry(manifest_rel)
                .and_modify(|allow_external| *allow_external = true)
                .or_insert(true);
        }
    }

    let mut catalog = RustWorkspaceCatalog::default();
    for (manifest_rel, allow_external) in manifests {
        let Some(descriptor) = load_rust_crate_descriptor(root, &manifest_rel) else {
            continue;
        };
        if allow_external {
            catalog.crate_names.insert(descriptor.crate_name.clone());
        }
        catalog.descriptors.push(descriptor);
    }
    catalog
}

fn find_nearest_cargo_manifest_rel(root: &Path, source_path: &Path) -> Option<String> {
    let mut directory = source_path.parent()?.to_path_buf();

    loop {
        let manifest_path = directory.join("Cargo.toml");
        if manifest_path.is_file() {
            return Some(normalize_relative_path(
                manifest_path.strip_prefix(root).unwrap_or(&manifest_path),
            ));
        }
        if directory == root || !directory.pop() {
            break;
        }
    }

    None
}

fn resolve_rust_workspace_member_manifest_rel(root: &Path, member: &str) -> Option<String> {
    let candidate = root.join(member);
    let manifest_path =
        if candidate.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml") {
            candidate
        } else {
            candidate.join("Cargo.toml")
        };
    if !manifest_path.is_file() {
        return None;
    }

    Some(normalize_relative_path(
        manifest_path.strip_prefix(root).unwrap_or(&manifest_path),
    ))
}

fn load_rust_crate_descriptor(root: &Path, manifest_rel: &str) -> Option<RustCrateDescriptor> {
    let manifest_path = root.join(manifest_rel);
    let manifest_dir = manifest_path.parent()?;
    let manifest_source = fs::read_to_string(&manifest_path).ok()?;
    let manifest: CargoManifestToml = toml::from_str(&manifest_source).ok()?;
    let crate_name = manifest
        .lib
        .as_ref()
        .and_then(|section| section.name.clone())
        .or_else(|| {
            manifest
                .package
                .as_ref()
                .map(|section| section.name.clone())
        })?
        .replace('-', "_");
    let crate_root_rel_path = manifest
        .lib
        .as_ref()
        .and_then(|section| section.path.as_deref())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("src/lib.rs"));
    let source_root_path = crate_root_rel_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let source_root_abs = manifest_dir.join(source_root_path);
    let crate_root_abs = manifest_dir.join(&crate_root_rel_path);

    Some(RustCrateDescriptor {
        crate_name,
        source_root_rel: normalize_relative_path(
            source_root_abs
                .strip_prefix(root)
                .unwrap_or(&source_root_abs),
        ),
        crate_root_rel: normalize_relative_path(
            crate_root_abs.strip_prefix(root).unwrap_or(&crate_root_abs),
        ),
    })
}

fn collect_rust_symbol_signatures(
    items: &[Item],
    module_path: &[String],
    crate_alias: Option<&str>,
    rust_crate_names: &BTreeSet<String>,
    symbol_signatures_by_path: &mut BTreeMap<String, Vec<String>>,
) {
    let module_prefix = rust_module_prefix(module_path);

    for item in items {
        match item {
            Item::Fn(item_fn) if is_public(&item_fn.vis) => {
                let signature = format!(
                    "rust:fn {module_prefix}{}",
                    format_rust_signature(&item_fn.sig)
                );
                let symbol_path = join_module_path(module_path, &item_fn.sig.ident.to_string());
                insert_rust_symbol_signature_variant(
                    symbol_signatures_by_path,
                    &symbol_path,
                    signature,
                    crate_alias,
                );
            }
            Item::Struct(item_struct) if is_public(&item_struct.vis) => {
                let signature = format!(
                    "rust:struct {module_prefix}{}{}",
                    item_struct.ident,
                    normalize_signature(&item_struct.generics.to_token_stream().to_string())
                );
                let symbol_path = join_module_path(module_path, &item_struct.ident.to_string());
                insert_rust_symbol_signature_variant(
                    symbol_signatures_by_path,
                    &symbol_path,
                    signature,
                    crate_alias,
                );
            }
            Item::Enum(item_enum) if is_public(&item_enum.vis) => {
                let signature = format!(
                    "rust:enum {module_prefix}{}{}",
                    item_enum.ident,
                    normalize_signature(&item_enum.generics.to_token_stream().to_string())
                );
                let symbol_path = join_module_path(module_path, &item_enum.ident.to_string());
                insert_rust_symbol_signature_variant(
                    symbol_signatures_by_path,
                    &symbol_path,
                    signature,
                    crate_alias,
                );
            }
            Item::Trait(item_trait) if is_public(&item_trait.vis) => {
                let symbol_path = join_module_path(module_path, &item_trait.ident.to_string());
                insert_rust_symbol_signature_variant(
                    symbol_signatures_by_path,
                    &symbol_path,
                    format!("rust:trait {symbol_path}"),
                    crate_alias,
                );
                for trait_item in &item_trait.items {
                    if let TraitItem::Fn(method) = trait_item {
                        insert_rust_symbol_signature_variant(
                            symbol_signatures_by_path,
                            &symbol_path,
                            format!(
                                "rust:trait-fn {symbol_path}::{}",
                                format_rust_signature(&method.sig)
                            ),
                            crate_alias,
                        );
                    }
                }
            }
            Item::Type(item_type) if is_public(&item_type.vis) => {
                let symbol_path = join_module_path(module_path, &item_type.ident.to_string());
                let ty = normalize_signature(&item_type.ty.to_token_stream().to_string());
                insert_rust_symbol_signature_variant(
                    symbol_signatures_by_path,
                    &symbol_path,
                    format!("rust:type {symbol_path}={ty}"),
                    crate_alias,
                );
            }
            Item::Const(item_const) if is_public(&item_const.vis) => {
                let symbol_path = join_module_path(module_path, &item_const.ident.to_string());
                let ty = normalize_signature(&item_const.ty.to_token_stream().to_string());
                insert_rust_symbol_signature_variant(
                    symbol_signatures_by_path,
                    &symbol_path,
                    format!("rust:const {symbol_path}:{ty}"),
                    crate_alias,
                );
            }
            Item::Static(item_static) if is_public(&item_static.vis) => {
                let symbol_path = join_module_path(module_path, &item_static.ident.to_string());
                let ty = normalize_signature(&item_static.ty.to_token_stream().to_string());
                insert_rust_symbol_signature_variant(
                    symbol_signatures_by_path,
                    &symbol_path,
                    format!("rust:static {symbol_path}:{ty}"),
                    crate_alias,
                );
            }
            Item::Impl(item_impl) => {
                let Some(symbol_path) = resolve_rust_type_symbol_path(
                    module_path,
                    &item_impl.self_ty,
                    rust_crate_names,
                ) else {
                    continue;
                };
                for impl_item in &item_impl.items {
                    if let ImplItem::Fn(method) = impl_item {
                        if !is_public(&method.vis) {
                            continue;
                        }
                        insert_rust_symbol_signature_variant(
                            symbol_signatures_by_path,
                            &symbol_path,
                            format!(
                                "rust:impl-fn {}::{}",
                                render_rust_impl_scope(&symbol_path, &item_impl.self_ty),
                                format_rust_signature(&method.sig)
                            ),
                            crate_alias,
                        );
                    }
                }
            }
            Item::Mod(item_mod) => {
                let mut nested_module = module_path.to_vec();
                nested_module.push(item_mod.ident.to_string());
                if let Some((_, nested_items)) = &item_mod.content {
                    collect_rust_symbol_signatures(
                        nested_items,
                        &nested_module,
                        crate_alias,
                        rust_crate_names,
                        symbol_signatures_by_path,
                    );
                }
            }
            _ => {}
        }
    }
}

fn collect_rust_reexported_signatures(
    items: &[Item],
    current_module: &[String],
    current_module_public: bool,
    rust_crate_names: &BTreeSet<String>,
    symbol_signatures_by_path: &BTreeMap<String, Vec<String>>,
    signatures: &mut BTreeSet<String>,
) {
    for item in items {
        match item {
            Item::Use(item_use) if current_module_public && is_public(&item_use.vis) => {
                collect_rust_use_tree_signatures(
                    &item_use.tree,
                    &[],
                    current_module,
                    rust_crate_names,
                    symbol_signatures_by_path,
                    signatures,
                );
            }
            Item::Mod(item_mod) if current_module_public => {
                let mut nested_module = current_module.to_vec();
                nested_module.push(item_mod.ident.to_string());
                if let Some((_, nested_items)) = &item_mod.content {
                    collect_rust_reexported_signatures(
                        nested_items,
                        &nested_module,
                        is_public(&item_mod.vis),
                        rust_crate_names,
                        symbol_signatures_by_path,
                        signatures,
                    );
                }
            }
            _ => {}
        }
    }
}

fn collect_rust_reexported_symbols(
    items: &[Item],
    current_module: &[String],
    current_module_public: bool,
    lookup: &RustReexportLookup<'_>,
    pending: &mut BTreeSet<RustReexportedSymbol>,
) {
    for item in items {
        match item {
            Item::Use(item_use) if current_module_public && is_public(&item_use.vis) => {
                collect_rust_use_tree_symbols(&item_use.tree, &[], current_module, lookup, pending);
            }
            Item::Mod(item_mod) if current_module_public => {
                let mut nested_module = current_module.to_vec();
                nested_module.push(item_mod.ident.to_string());
                if let Some((_, nested_items)) = &item_mod.content {
                    collect_rust_reexported_symbols(
                        nested_items,
                        &nested_module,
                        is_public(&item_mod.vis),
                        lookup,
                        pending,
                    );
                }
            }
            _ => {}
        }
    }
}

fn collect_rust_use_tree_signatures(
    tree: &UseTree,
    prefix: &[String],
    current_module: &[String],
    rust_crate_names: &BTreeSet<String>,
    symbol_signatures_by_path: &BTreeMap<String, Vec<String>>,
    signatures: &mut BTreeSet<String>,
) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix.to_vec();
            next.push(path.ident.to_string());
            collect_rust_use_tree_signatures(
                &path.tree,
                &next,
                current_module,
                rust_crate_names,
                symbol_signatures_by_path,
                signatures,
            );
        }
        UseTree::Name(name) => {
            if name.ident == "self" {
                return;
            }
            let mut source = prefix.to_vec();
            source.push(name.ident.to_string());
            resolve_rust_reexport_target(
                &source,
                None,
                false,
                current_module,
                rust_crate_names,
                symbol_signatures_by_path,
                signatures,
            );
        }
        UseTree::Rename(rename) => {
            let mut source = prefix.to_vec();
            source.push(rename.ident.to_string());
            resolve_rust_reexport_target(
                &source,
                Some(rename.rename.to_string()),
                false,
                current_module,
                rust_crate_names,
                symbol_signatures_by_path,
                signatures,
            );
        }
        UseTree::Glob(_) => {
            resolve_rust_reexport_target(
                prefix,
                None,
                true,
                current_module,
                rust_crate_names,
                symbol_signatures_by_path,
                signatures,
            );
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_rust_use_tree_signatures(
                    item,
                    prefix,
                    current_module,
                    rust_crate_names,
                    symbol_signatures_by_path,
                    signatures,
                );
            }
        }
    }
}

fn collect_rust_use_tree_symbols(
    tree: &UseTree,
    prefix: &[String],
    current_module: &[String],
    lookup: &RustReexportLookup<'_>,
    pending: &mut BTreeSet<RustReexportedSymbol>,
) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix.to_vec();
            next.push(path.ident.to_string());
            collect_rust_use_tree_symbols(&path.tree, &next, current_module, lookup, pending);
        }
        UseTree::Name(name) => {
            if name.ident == "self" {
                return;
            }
            let mut source = prefix.to_vec();
            source.push(name.ident.to_string());
            resolve_rust_reexported_symbol(&source, None, false, current_module, lookup, pending);
        }
        UseTree::Rename(rename) => {
            let mut source = prefix.to_vec();
            source.push(rename.ident.to_string());
            resolve_rust_reexported_symbol(
                &source,
                Some(rename.rename.to_string()),
                false,
                current_module,
                lookup,
                pending,
            );
        }
        UseTree::Glob(_) => {
            resolve_rust_reexported_symbol(prefix, None, true, current_module, lookup, pending);
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_rust_use_tree_symbols(item, prefix, current_module, lookup, pending);
            }
        }
    }
}

fn resolve_rust_reexport_target(
    source: &[String],
    alias: Option<String>,
    is_glob: bool,
    current_module: &[String],
    rust_crate_names: &BTreeSet<String>,
    symbol_signatures_by_path: &BTreeMap<String, Vec<String>>,
    signatures: &mut BTreeSet<String>,
) {
    let Some(resolved_source) = resolve_rust_path(current_module, source, rust_crate_names) else {
        return;
    };
    let source_path = resolved_source.join("::");

    if is_glob {
        let prefix = format!("{source_path}::");
        for (symbol_path, symbol_signatures) in symbol_signatures_by_path {
            if !(symbol_path == &source_path || symbol_path.starts_with(&prefix)) {
                continue;
            }
            let remainder = symbol_path
                .strip_prefix(&prefix)
                .or_else(|| symbol_path.strip_prefix(&source_path))
                .unwrap_or(symbol_path)
                .trim_start_matches("::");
            if remainder.is_empty() {
                continue;
            }
            let export_path = join_module_path(current_module, remainder);
            emit_rust_reexported_signatures(
                symbol_path,
                &export_path,
                symbol_signatures,
                signatures,
            );
        }
        return;
    }

    let export_name = alias.unwrap_or_else(|| {
        resolved_source
            .last()
            .cloned()
            .unwrap_or_else(|| source_path.clone())
    });
    let export_path = join_module_path(current_module, &export_name);
    let Some(symbol_signatures) = symbol_signatures_by_path.get(&source_path) else {
        return;
    };
    emit_rust_reexported_signatures(&source_path, &export_path, symbol_signatures, signatures);
}

fn resolve_rust_reexported_symbol(
    source: &[String],
    alias: Option<String>,
    is_glob: bool,
    current_module: &[String],
    lookup: &RustReexportLookup<'_>,
    pending: &mut BTreeSet<RustReexportedSymbol>,
) {
    let Some(resolved_source) = resolve_rust_path(current_module, source, lookup.rust_crate_names)
    else {
        return;
    };
    let source_path = resolved_source.join("::");

    if is_glob {
        let prefix = format!("{source_path}::");
        for (symbol_path, symbol_signatures) in lookup.symbol_signatures_by_path {
            if !(symbol_path == &source_path || symbol_path.starts_with(&prefix)) {
                continue;
            }
            let remainder = symbol_path
                .strip_prefix(&prefix)
                .or_else(|| symbol_path.strip_prefix(&source_path))
                .unwrap_or(symbol_path)
                .trim_start_matches("::");
            if remainder.is_empty() {
                continue;
            }
            let export_path = join_module_path(current_module, remainder);
            queue_rust_reexported_symbols(
                symbol_path,
                &export_path,
                symbol_signatures,
                lookup.crate_alias,
                pending,
            );
        }
        return;
    }

    let export_name = alias.unwrap_or_else(|| {
        resolved_source
            .last()
            .cloned()
            .unwrap_or_else(|| source_path.clone())
    });
    let export_path = join_module_path(current_module, &export_name);
    let Some(symbol_signatures) = lookup.symbol_signatures_by_path.get(&source_path) else {
        return;
    };
    queue_rust_reexported_symbols(
        &source_path,
        &export_path,
        symbol_signatures,
        lookup.crate_alias,
        pending,
    );
}

fn emit_rust_reexported_signatures(
    source_path: &str,
    export_path: &str,
    symbol_signatures: &[String],
    signatures: &mut BTreeSet<String>,
) {
    for signature in symbol_signatures {
        signatures.insert(signature.replacen(source_path, export_path, 1));
    }
}

fn queue_rust_reexported_symbols(
    source_path: &str,
    export_path: &str,
    symbol_signatures: &[String],
    crate_alias: Option<&str>,
    pending: &mut BTreeSet<RustReexportedSymbol>,
) {
    for signature in symbol_signatures {
        pending.insert(RustReexportedSymbol {
            symbol_path: export_path.to_string(),
            signature: signature.replacen(source_path, export_path, 1),
            crate_alias: crate_alias.map(str::to_string),
        });
    }
}

fn insert_rust_symbol_signature(
    symbol_signatures_by_path: &mut BTreeMap<String, Vec<String>>,
    symbol_path: &str,
    signature: String,
) -> bool {
    let signatures = symbol_signatures_by_path
        .entry(symbol_path.to_string())
        .or_default();
    if signatures.contains(&signature) {
        return false;
    }
    signatures.push(signature);
    true
}

fn insert_rust_symbol_signature_variant(
    symbol_signatures_by_path: &mut BTreeMap<String, Vec<String>>,
    symbol_path: &str,
    signature: String,
    crate_alias: Option<&str>,
) -> bool {
    let mut changed =
        insert_rust_symbol_signature(symbol_signatures_by_path, symbol_path, signature.clone());

    if let Some(crate_alias) = crate_alias {
        let qualified_symbol_path = format!("{crate_alias}::{symbol_path}");
        changed |= insert_rust_symbol_signature(
            symbol_signatures_by_path,
            &qualified_symbol_path,
            signature.replacen(symbol_path, &qualified_symbol_path, 1),
        );
    }

    changed
}

fn collect_rust_public_module_edges(
    items: &[Item],
    module_dir: &str,
    known_rust_files: &BTreeSet<String>,
    edges: &mut BTreeSet<String>,
) {
    for item in items {
        let Item::Mod(item_mod) = item else {
            continue;
        };
        if !is_public(&item_mod.vis) {
            continue;
        }

        let module_name = item_mod.ident.to_string();
        let nested_module_dir = join_rust_module_dir(module_dir, &module_name);
        if let Some((_, nested_items)) = &item_mod.content {
            collect_rust_public_module_edges(
                nested_items,
                &nested_module_dir,
                known_rust_files,
                edges,
            );
            continue;
        }

        if let Some(rel_path) =
            resolve_rust_external_module_file(module_dir, &module_name, known_rust_files)
        {
            edges.insert(rel_path);
        }
    }
}

fn resolve_rust_external_module_file(
    module_dir: &str,
    module_name: &str,
    known_rust_files: &BTreeSet<String>,
) -> Option<String> {
    let base_dir = if module_dir.is_empty() {
        PathBuf::new()
    } else {
        PathBuf::from(module_dir)
    };
    let direct_file = normalize_relative_path(&base_dir.join(format!("{module_name}.rs")));
    if known_rust_files.contains(&direct_file) {
        return Some(direct_file);
    }

    let mod_file = normalize_relative_path(&base_dir.join(module_name).join("mod.rs"));
    if known_rust_files.contains(&mod_file) {
        return Some(mod_file);
    }

    None
}

fn rust_module_directory(rel_path: &Path, descriptor: Option<&RustCrateDescriptor>) -> String {
    let parent = rel_path.parent().unwrap_or_else(|| Path::new(""));
    let file_name = rel_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let stem = rel_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let module_dir = if is_rust_module_root_file(rel_path, descriptor)
        || matches!(file_name, "lib.rs" | "main.rs" | "mod.rs")
    {
        parent.to_path_buf()
    } else {
        parent.join(stem)
    };
    normalize_relative_path(&module_dir)
}

fn rust_file_module_path(rel_path: &Path, descriptor: Option<&RustCrateDescriptor>) -> Vec<String> {
    let crate_relative = rust_path_after_source_root(rel_path, descriptor);
    let file_name = crate_relative
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let mut parts: Vec<String> = crate_relative
        .parent()
        .into_iter()
        .flat_map(|parent| parent.iter())
        .filter_map(|component| component.to_str().map(str::to_string))
        .collect();

    if is_rust_module_root_file(rel_path, descriptor)
        || matches!(file_name, "lib.rs" | "main.rs" | "mod.rs")
    {
        return parts;
    }

    if let Some(stem) = crate_relative.file_stem().and_then(|name| name.to_str()) {
        parts.push(stem.to_string());
    }
    parts
}

fn rust_path_after_source_root(
    rel_path: &Path,
    descriptor: Option<&RustCrateDescriptor>,
) -> PathBuf {
    if let Some(descriptor) = descriptor {
        let source_root = Path::new(&descriptor.source_root_rel);
        if let Ok(stripped) = rel_path.strip_prefix(source_root) {
            return stripped.to_path_buf();
        }
    }

    let components: Vec<String> = rel_path
        .iter()
        .filter_map(|component| component.to_str().map(str::to_string))
        .collect();
    if let Some(index) = components.iter().rposition(|component| component == "src") {
        return components[index + 1..].iter().collect();
    }
    components.iter().collect()
}

fn join_rust_module_dir(module_dir: &str, module_name: &str) -> String {
    if module_dir.is_empty() {
        module_name.to_string()
    } else {
        format!("{module_dir}/{module_name}")
    }
}

fn rust_module_prefix(module_path: &[String]) -> String {
    if module_path.is_empty() {
        String::new()
    } else {
        format!("{}::", module_path.join("::"))
    }
}

fn join_module_path(module_path: &[String], name: &str) -> String {
    if module_path.is_empty() {
        name.to_string()
    } else if name.is_empty() {
        module_path.join("::")
    } else {
        format!("{}::{name}", module_path.join("::"))
    }
}

fn resolve_rust_type_symbol_path(
    module_path: &[String],
    ty: &Type,
    rust_crate_names: &BTreeSet<String>,
) -> Option<String> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    if type_path.qself.is_some() {
        return None;
    }

    let segments: Vec<String> = type_path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    let resolved = resolve_rust_path(module_path, &segments, rust_crate_names)?;
    Some(resolved.join("::"))
}

fn resolve_rust_path(
    current_module: &[String],
    source: &[String],
    rust_crate_names: &BTreeSet<String>,
) -> Option<Vec<String>> {
    if source
        .first()
        .is_some_and(|segment| rust_crate_names.contains(segment))
    {
        return Some(source.to_vec());
    }

    let mut resolved = current_module.to_vec();
    let mut index = 0usize;

    if source.first().map(String::as_str) == Some("crate") {
        resolved.clear();
        index = 1;
    } else {
        while source.get(index).map(String::as_str) == Some("super") {
            resolved.pop()?;
            index += 1;
        }
        if source.get(index).map(String::as_str) == Some("self") {
            index += 1;
        }
    }

    resolved.extend(source[index..].iter().cloned());
    Some(resolved)
}

fn render_rust_impl_scope(symbol_path: &str, self_ty: &Type) -> String {
    let rendered = normalize_signature(&self_ty.to_token_stream().to_string());
    let simple_name = symbol_path.rsplit("::").next().unwrap_or(symbol_path);
    if simple_name.is_empty() {
        rendered
    } else {
        rendered.replacen(simple_name, symbol_path, 1)
    }
}

fn normalize_relative_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_matches('/')
        .to_string()
}

fn path_has_prefix(relative_path: &str, prefix: &str) -> bool {
    prefix.is_empty()
        || relative_path == prefix
        || relative_path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn is_rust_module_root_file(rel_path: &Path, descriptor: Option<&RustCrateDescriptor>) -> bool {
    descriptor
        .is_some_and(|descriptor| descriptor.crate_root_rel == normalize_relative_path(rel_path))
}

fn extract_named_tags(
    comments: &[String],
    regex: &Regex,
    primary_group: &str,
    fallback_group: &str,
) -> Vec<String> {
    let mut values = Vec::new();

    for comment in comments {
        for capture in regex.captures_iter(comment) {
            let tag = capture
                .name(primary_group)
                .or_else(|| capture.name(fallback_group))
                .map(|value| value.as_str().trim().to_string())
                .unwrap_or_default();

            if !tag.is_empty() {
                values.push(tag);
            }
        }
    }

    values
}

fn extract_public_api(
    language: SourceLanguage,
    rel_path: &str,
    source: &str,
    masked_code: &str,
    context: PublicApiExtractionContext<'_>,
) -> Vec<String> {
    if language == SourceLanguage::Rust {
        let mut signatures = extract_rust_public_api(source).unwrap_or_default();
        if context.scan_policy.rust_export_following == RustExportFollowing::PublicModules {
            signatures.extend(
                context
                    .rust_module_index
                    .reexported_signatures_for(rel_path)
                    .iter()
                    .cloned(),
            );
        }
        signatures.sort();
        signatures.dedup();
        if !signatures.is_empty() {
            return signatures;
        }
    }

    if let Some(tree_sitter_signatures) = extract_tree_sitter_public_api(
        language,
        rel_path,
        source,
        context.scan_policy,
        context.ts_module_index,
        context.python_module_index,
    ) {
        return tree_sitter_signatures;
    }

    let regex_source = if language == SourceLanguage::Dart {
        source
    } else {
        masked_code
    };
    extract_regex_public_api(language, regex_source, context.regex)
}

fn extract_tree_sitter_public_api(
    language: SourceLanguage,
    rel_path: &str,
    source: &str,
    scan_policy: &ScanPolicy,
    ts_module_index: &adapters::TsModuleIndex,
    python_module_index: &adapters::PythonModuleIndex,
) -> Option<Vec<String>> {
    let grammar = language.tree_sitter_language()?;
    let mut parser = TreeSitterParser::new();
    parser.set_language(&grammar).ok()?;
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();

    let mut signatures = adapters::extract_tree_sitter_public_api(
        language,
        root,
        source,
        rel_path,
        adapters::TreeSitterExtractionContext {
            ts_module_index: Some(ts_module_index),
            python_module_index: Some(python_module_index),
            ruby_export_following: scan_policy.ruby_export_following,
            lua_export_following: scan_policy.lua_export_following,
        },
    );
    signatures.sort();
    signatures.dedup();
    Some(signatures)
}

fn extract_tree_sitter_prefix_signature(
    node: Node<'_>,
    source: &str,
    field_name: &str,
) -> Option<String> {
    let end = node
        .child_by_field_name(field_name)
        .map(|field| field.start_byte())
        .unwrap_or_else(|| node.end_byte());
    node_text_range(source, node.start_byte(), end)
        .map(normalize_tree_sitter_signature)
        .filter(|signature| !signature.is_empty())
}

fn extract_tree_sitter_prefix_before_named_child(
    node: Node<'_>,
    source: &str,
    stop_kinds: &[&str],
) -> Option<String> {
    let end = named_children(node)
        .into_iter()
        .find(|child| stop_kinds.contains(&child.kind()))
        .map(|child| child.start_byte())
        .unwrap_or_else(|| node.end_byte());
    node_text_range(source, node.start_byte(), end)
        .map(normalize_tree_sitter_signature)
        .filter(|signature| !signature.is_empty())
}

fn extract_tree_sitter_prefix_before_fields(
    node: Node<'_>,
    source: &str,
    stop_fields: &[&str],
) -> Option<String> {
    let end = stop_fields
        .iter()
        .filter_map(|field_name| {
            node.child_by_field_name(field_name)
                .map(|field| field.start_byte())
        })
        .min()
        .unwrap_or_else(|| node.end_byte());

    node_text_range(source, node.start_byte(), end)
        .map(normalize_tree_sitter_signature)
        .filter(|signature| !signature.is_empty())
}

fn normalize_export_statement_signature(raw: &str) -> String {
    normalize_tree_sitter_signature(raw)
}

fn normalize_tree_sitter_signature(raw: &str) -> String {
    let mut normalized =
        normalize_signature(raw.trim().trim_end_matches(';').trim_end_matches(':'));
    for (from, to) in [(",)", ")"), (", }", " }"), (",]", "]")] {
        normalized = normalized.replace(from, to);
    }
    normalized
}

fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut children = Vec::new();
    for index in 0..node.named_child_count() {
        if let Some(child) = node.named_child(index) {
            children.push(child);
        }
    }
    children
}

fn children_by_field_name<'a>(node: Node<'a>, field_name: &str) -> Vec<Node<'a>> {
    let mut cursor = node.walk();
    node.children_by_field_name(field_name, &mut cursor)
        .collect()
}

fn is_exported_tree_sitter_name(node: Node<'_>, source: &str) -> bool {
    node.child_by_field_name("name")
        .and_then(|child| node_text(child, source))
        .map(|name| {
            name.chars()
                .next()
                .map(|first| first.is_ascii_uppercase())
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn is_public_python_name(node: Node<'_>, source: &str) -> bool {
    node.child_by_field_name("name")
        .and_then(|child| node_text(child, source))
        .map(|name| !name.starts_with('_'))
        .unwrap_or(false)
}

fn has_tree_sitter_keyword(
    node: Node<'_>,
    source: &str,
    stop_field_name: &str,
    keyword: &str,
) -> bool {
    extract_tree_sitter_prefix_signature(node, source, stop_field_name)
        .map(|signature| contains_signature_keyword(&signature, keyword))
        .unwrap_or(false)
}

fn normalize_kotlin_signature(signature: &str) -> Option<String> {
    if contains_signature_keyword(signature, "private")
        || contains_signature_keyword(signature, "protected")
        || contains_signature_keyword(signature, "internal")
    {
        return None;
    }

    trim_signature_to_keywords(
        signature,
        &[
            "public",
            "data",
            "sealed",
            "enum",
            "annotation",
            "value",
            "suspend",
            "class",
            "interface",
            "object",
            "fun",
        ],
    )
}

fn normalize_php_type_signature(signature: &str) -> Option<String> {
    trim_signature_to_keywords(
        signature,
        &[
            "abstract",
            "final",
            "readonly",
            "class",
            "interface",
            "trait",
            "enum",
        ],
    )
}

fn normalize_php_function_signature(signature: &str) -> Option<String> {
    trim_signature_to_keywords(signature, &["function"])
}

fn normalize_php_method_signature(signature: &str, inside_interface: bool) -> Option<String> {
    if contains_any_signature_keyword(signature, &["private", "protected"]) {
        return None;
    }

    if inside_interface {
        return trim_signature_to_keywords(signature, &["function"]);
    }

    if contains_signature_keyword(signature, "public") {
        return trim_signature_to_keywords(signature, &["public"]);
    }

    None
}

fn extract_tree_sitter_php_property_signatures(
    node: Node<'_>,
    source: &str,
    inside_interface: bool,
) -> Vec<String> {
    let visibility = php_visibility_modifier(node, source).or_else(|| {
        if php_has_modifier(node, "var_modifier") || inside_interface {
            Some("public".to_string())
        } else {
            None
        }
    });
    if visibility.as_deref() != Some("public") {
        return Vec::new();
    }

    let mut prefix_parts = vec![visibility.expect("public visibility already checked")];
    if php_has_modifier(node, "static_modifier") {
        prefix_parts.push("static".to_string());
    }
    if php_has_modifier(node, "readonly_modifier") {
        prefix_parts.push("readonly".to_string());
    }
    if let Some(type_annotation) = node
        .child_by_field_name("type")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
    {
        prefix_parts.push(type_annotation);
    }

    let prefix = prefix_parts.join(" ");
    let mut signatures = Vec::new();
    for child in named_children(node) {
        if child.kind() != "property_element" {
            continue;
        }

        let Some(name) = child
            .child_by_field_name("name")
            .and_then(|name| node_text(name, source))
            .map(str::trim)
        else {
            continue;
        };
        if name.is_empty() {
            continue;
        }

        let signature = normalize_signature(&format!("{prefix} {name}"));
        if !signature.is_empty() {
            signatures.push(format!("php:{signature}"));
        }
    }

    signatures
}

fn extract_tree_sitter_php_const_signatures(
    node: Node<'_>,
    source: &str,
    inside_interface: bool,
) -> Vec<String> {
    let visibility = php_visibility_modifier(node, source).or_else(|| {
        if inside_interface {
            Some("public".to_string())
        } else {
            None
        }
    });
    if matches!(visibility.as_deref(), Some("private" | "protected")) {
        return Vec::new();
    }

    let mut prefix_parts = Vec::new();
    if let Some(visibility) = visibility {
        prefix_parts.push(visibility);
    }
    if php_has_modifier(node, "final_modifier") {
        prefix_parts.push("final".to_string());
    }
    prefix_parts.push("const".to_string());
    if let Some(type_annotation) = node
        .child_by_field_name("type")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
    {
        prefix_parts.push(type_annotation);
    }

    let prefix = prefix_parts.join(" ");
    let mut signatures = Vec::new();
    for child in named_children(node) {
        if child.kind() != "const_element" {
            continue;
        }

        let Some(name) = named_children(child)
            .into_iter()
            .find(|named_child| named_child.kind() == "name")
            .and_then(|name| node_text(name, source))
            .map(str::trim)
        else {
            continue;
        };
        if name.is_empty() {
            continue;
        }

        let signature = normalize_signature(&format!("{prefix} {name}"));
        if !signature.is_empty() {
            signatures.push(format!("php:{signature}"));
        }
    }

    signatures
}

fn php_visibility_modifier(node: Node<'_>, source: &str) -> Option<String> {
    named_children(node)
        .into_iter()
        .find(|child| child.kind() == "visibility_modifier")
        .and_then(|child| node_text(child, source))
        .map(normalize_tree_sitter_signature)
        .filter(|value| !value.is_empty())
}

fn php_has_modifier(node: Node<'_>, modifier_kind: &str) -> bool {
    named_children(node)
        .into_iter()
        .any(|child| child.kind() == modifier_kind)
}

fn normalize_swift_signature(signature: &str) -> Option<String> {
    if !contains_any_signature_keyword(signature, &["public", "open"]) {
        return None;
    }

    trim_signature_to_keywords(signature, &["public", "open"])
}

fn normalize_swift_protocol_member_signature(
    signature: &str,
    inherited_visibility: Option<&str>,
) -> Option<String> {
    if let Some(signature) = normalize_swift_signature(signature) {
        return Some(signature);
    }

    let inherited_visibility = inherited_visibility?;
    let normalized = normalize_tree_sitter_signature(signature);
    if normalized.is_empty() {
        return None;
    }

    Some(format!("{inherited_visibility} {normalized}"))
}

fn swift_visibility_keyword(signature: &str) -> Option<&'static str> {
    if contains_signature_keyword(signature, "open") {
        Some("open")
    } else if contains_signature_keyword(signature, "public") {
        Some("public")
    } else {
        None
    }
}

fn trim_signature_to_keywords(signature: &str, keywords: &[&str]) -> Option<String> {
    let start = keywords
        .iter()
        .filter_map(|keyword| find_signature_keyword(signature, keyword))
        .min()
        .unwrap_or(0);
    let trimmed = normalize_tree_sitter_signature(signature.get(start..)?.trim());
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn find_signature_keyword(signature: &str, keyword: &str) -> Option<usize> {
    let bytes = signature.as_bytes();
    let keyword_bytes = keyword.as_bytes();
    if keyword_bytes.is_empty() || keyword_bytes.len() > bytes.len() {
        return None;
    }

    for index in 0..=bytes.len() - keyword_bytes.len() {
        if &bytes[index..index + keyword_bytes.len()] != keyword_bytes {
            continue;
        }

        let left_ok = index == 0 || !is_signature_word_byte(bytes[index - 1]);
        let right_index = index + keyword_bytes.len();
        let right_ok = right_index == bytes.len() || !is_signature_word_byte(bytes[right_index]);
        if left_ok && right_ok {
            return Some(index);
        }
    }

    None
}

fn contains_signature_keyword(signature: &str, keyword: &str) -> bool {
    find_signature_keyword(signature, keyword).is_some()
}

fn contains_any_signature_keyword(signature: &str, keywords: &[&str]) -> bool {
    keywords
        .iter()
        .any(|keyword| contains_signature_keyword(signature, keyword))
}

fn is_signature_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    node_text_range(source, node.start_byte(), node.end_byte())
}

fn node_text_range(source: &str, start: usize, end: usize) -> Option<&str> {
    source.get(start..end)
}

fn dart_ident_public(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('_')
}

fn dart_qualified_library_prefix(library_prefix: &str, tail: &str) -> String {
    if library_prefix.is_empty() {
        tail.to_string()
    } else {
        format!("{library_prefix}{tail}")
    }
}

fn extract_dart_public_api(source: &str, regex: &ApiRegexPack) -> Vec<String> {
    let mut signatures = Vec::new();
    let mut library_prefix = String::new();

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || is_comment_line(trimmed, "dart") {
            continue;
        }

        if let Some(cap) = regex.dart_library.captures(trimmed) {
            if let Some(lib) = cap.name("lib") {
                let mut value = lib.as_str().to_string();
                if !value.ends_with('.') {
                    value.push('.');
                }
                library_prefix = value;
            }
        }

        if let Some(cap) = regex.dart_class.captures(trimmed) {
            let name = cap.name("name").map(|value| value.as_str()).unwrap_or("");
            if dart_ident_public(name) {
                let tail = normalize_signature(&format!("class {name}"));
                if !tail.is_empty() {
                    signatures.push(format!(
                        "dart:type {}",
                        dart_qualified_library_prefix(&library_prefix, &tail)
                    ));
                }
            }
        }

        if let Some(cap) = regex.dart_mixin.captures(trimmed) {
            let name = cap.name("name").map(|value| value.as_str()).unwrap_or("");
            if dart_ident_public(name) {
                let tail = normalize_signature(&format!("mixin {name}"));
                if !tail.is_empty() {
                    signatures.push(format!(
                        "dart:type {}",
                        dart_qualified_library_prefix(&library_prefix, &tail)
                    ));
                }
            }
        }

        if let Some(cap) = regex.dart_enum.captures(trimmed) {
            let name = cap.name("name").map(|value| value.as_str()).unwrap_or("");
            if dart_ident_public(name) {
                let tail = normalize_signature(&format!("enum {name}"));
                if !tail.is_empty() {
                    signatures.push(format!(
                        "dart:type {}",
                        dart_qualified_library_prefix(&library_prefix, &tail)
                    ));
                }
            }
        }

        if let Some(cap) = regex.dart_extension_type.captures(trimmed) {
            let name = cap.name("name").map(|value| value.as_str()).unwrap_or("");
            if dart_ident_public(name) {
                let tail = normalize_signature(&format!("extension type {name}"));
                if !tail.is_empty() {
                    signatures.push(format!(
                        "dart:type {}",
                        dart_qualified_library_prefix(&library_prefix, &tail)
                    ));
                }
            }
        }

        if let Some(cap) = regex.dart_extension.captures(trimmed) {
            let on = cap
                .name("on")
                .map(|value| normalize_signature(value.as_str()))
                .unwrap_or_default();
            if !on.is_empty() {
                if let Some(named) = cap.name("named").map(|value| value.as_str()) {
                    if dart_ident_public(named) {
                        let tail = normalize_signature(&format!("extension {named} on {on}"));
                        if !tail.is_empty() {
                            signatures.push(format!(
                                "dart:type {}",
                                dart_qualified_library_prefix(&library_prefix, &tail)
                            ));
                        }
                    }
                } else {
                    let tail = normalize_signature(&format!("extension on {on}"));
                    if !tail.is_empty() {
                        signatures.push(format!(
                            "dart:type {}",
                            dart_qualified_library_prefix(&library_prefix, &tail)
                        ));
                    }
                }
            }
        }

        if let Some(cap) = regex.dart_typedef_equals.captures(trimmed) {
            let name = cap.name("name").map(|value| value.as_str()).unwrap_or("");
            if dart_ident_public(name) {
                let tail = normalize_signature(&format!("typedef {name} ="));
                if !tail.is_empty() {
                    signatures.push(format!(
                        "dart:type {}",
                        dart_qualified_library_prefix(&library_prefix, &tail)
                    ));
                }
            }
        }

        if let Some(cap) = regex.dart_static_const.captures(trimmed) {
            let name = cap.name("name").map(|value| value.as_str()).unwrap_or("");
            if dart_ident_public(name) {
                let ty = cap.name("ty").map(|value| value.as_str()).unwrap_or("");
                let tail = normalize_signature(&format!("static const {ty} {name} ="));
                if !tail.is_empty() {
                    signatures.push(format!(
                        "dart:field {}",
                        dart_qualified_library_prefix(&library_prefix, &tail)
                    ));
                }
            }
        }

        if let Some(cap) = regex.dart_getter.captures(trimmed) {
            let ret = cap.name("ret").map(|value| value.as_str()).unwrap_or("").trim();
            let name = cap.name("name").map(|value| value.as_str()).unwrap_or("");
            if dart_ident_public(name) {
                let tail = normalize_signature(&format!("{ret} get {name}"));
                if !tail.is_empty() {
                    signatures.push(format!(
                        "dart:getter {}",
                        dart_qualified_library_prefix(&library_prefix, &tail)
                    ));
                }
            }
        }

        if let Some(cap) = regex.dart_setter.captures(trimmed) {
            let name = cap.name("name").map(|value| value.as_str()).unwrap_or("");
            if dart_ident_public(name) {
                let tail = normalize_signature(&format!("void set {name}"));
                if !tail.is_empty() {
                    signatures.push(format!(
                        "dart:setter {}",
                        dart_qualified_library_prefix(&library_prefix, &tail)
                    ));
                }
            }
        }

        if let Some(cap) = regex.dart_callable.captures(trimmed) {
            let ret = cap.name("ret").map(|value| value.as_str()).unwrap_or("").trim();
            let name = cap.name("name").map(|value| value.as_str()).unwrap_or("");
            let args = cap.name("args").map(|value| value.as_str()).unwrap_or("");
            if matches!(ret, "if" | "for" | "while" | "switch" | "catch" | "return") {
                continue;
            }
            if dart_ident_public(name) {
                let tail = normalize_signature(&format!("{ret} {name}({args})"));
                if !tail.is_empty() {
                    signatures.push(format!(
                        "dart:function {}",
                        dart_qualified_library_prefix(&library_prefix, &tail)
                    ));
                }
            }
        }

        if let Some(cap) = regex.dart_field.captures(trimmed) {
            let ty = cap.name("ty").map(|value| value.as_str()).unwrap_or("").trim();
            let name = cap.name("name").map(|value| value.as_str()).unwrap_or("");
            if matches!(ty, "if" | "for" | "while" | "return") {
                continue;
            }
            if dart_ident_public(name) {
                let tail = normalize_signature(&format!("{ty} {name}"));
                if !tail.is_empty() {
                    signatures.push(format!(
                        "dart:field {}",
                        dart_qualified_library_prefix(&library_prefix, &tail)
                    ));
                }
            }
        }
    }

    signatures.sort();
    signatures.dedup();
    signatures
}

fn extract_regex_public_api(
    language: SourceLanguage,
    source: &str,
    regex: &ApiRegexPack,
) -> Vec<String> {
    if language == SourceLanguage::Dart {
        return extract_dart_public_api(source, regex);
    }

    let mut signatures = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || is_comment_line(trimmed, language.extension_label()) {
            continue;
        }

        match language {
            SourceLanguage::TypeScript
            | SourceLanguage::Tsx
            | SourceLanguage::JavaScript
            | SourceLanguage::Jsx
            | SourceLanguage::Php
            | SourceLanguage::Ruby
            | SourceLanguage::Swift
            | SourceLanguage::Lua
            | SourceLanguage::Luau
            | SourceLanguage::Dart => {}
            SourceLanguage::Go => {
                if let Some(capture) = regex.go_func.captures(trimmed) {
                    let recv = capture
                        .name("recv")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let name = capture
                        .name("name")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let sig = capture
                        .name("sig")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let signature = normalize_signature(&format!("func {recv}{name}{sig}"));
                    if !signature.is_empty() {
                        signatures.push(format!("go:{signature}"));
                    }
                }
            }
            SourceLanguage::Python => {
                if let Some(capture) = regex.py_def.captures(trimmed) {
                    let name = capture
                        .name("name")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    if name.starts_with('_') {
                        continue;
                    }
                    let sig = capture
                        .name("sig")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let signature = normalize_signature(&format!("def {name}{sig}"));
                    if !signature.is_empty() {
                        signatures.push(format!("python:{signature}"));
                    }
                }
            }
            SourceLanguage::Java => {
                if let Some(capture) = regex.java_type.captures(trimmed) {
                    if let Some(name) = capture.name("name") {
                        signatures.push(format!("java:type {}", name.as_str()));
                    }
                }
                if let Some(capture) = regex.java_method.captures(trimmed) {
                    let name = capture
                        .name("name")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let sig = capture
                        .name("sig")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    signatures.push(format!("java:method {}{}", name, normalize_signature(sig)));
                }
            }
            SourceLanguage::Kotlin => {
                if let Some(capture) = regex.kt_decl.captures(trimmed) {
                    let kind = capture
                        .name("kind")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let name = capture
                        .name("name")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let rest = capture
                        .name("rest")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let signature = normalize_signature(&format!("{kind} {name} {rest}"));
                    if !signature.is_empty() {
                        signatures.push(format!("kotlin:{signature}"));
                    }
                }
            }
            SourceLanguage::Csharp => {
                if let Some(capture) = regex.cs_type.captures(trimmed) {
                    if let Some(name) = capture.name("name") {
                        signatures.push(format!("csharp:type {}", name.as_str()));
                    }
                }
                if let Some(capture) = regex.cs_method.captures(trimmed) {
                    let name = capture
                        .name("name")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    let sig = capture
                        .name("sig")
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    signatures.push(format!(
                        "csharp:method {}{}",
                        name,
                        normalize_signature(sig)
                    ));
                }
            }
            SourceLanguage::Rust => {}
        }
    }

    signatures.sort();
    signatures.dedup();
    signatures
}

fn lex_source(source: &str, language: SourceLanguage) -> LexedSource {
    match language.lex_strategy() {
        LexStrategy::CStyle => lex_c_style_source(source, language),
        LexStrategy::Python => lex_python_source(source),
        LexStrategy::Php => lex_php_source(source),
        LexStrategy::Ruby => lex_ruby_source(source),
        LexStrategy::LuaFamily => lex_lua_family_source(source),
    }
}

fn lex_c_style_source(source: &str, language: SourceLanguage) -> LexedSource {
    let bytes = source.as_bytes();
    let mut masked = bytes.to_vec();
    let mut comments = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if language == SourceLanguage::Rust {
            if let Some(end) = skip_rust_raw_string(bytes, i) {
                mask_range(&mut masked, bytes, i, end);
                i = end;
                continue;
            }
        }

        if language == SourceLanguage::Swift {
            if let Some(end) = skip_swift_string(bytes, i) {
                mask_range(&mut masked, bytes, i, end);
                i = end;
                continue;
            }
        }

        if uses_backtick_strings(language) && bytes[i] == b'`' {
            let end = skip_backtick_string(bytes, i);
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if uses_single_quote_strings(language) && bytes[i] == b'\'' {
            let end = skip_quoted_string(bytes, i, b'\'');
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if is_csharp_verbatim_string_start(bytes, i) {
            let end = skip_csharp_verbatim_string(bytes, i);
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'"' {
            let end = skip_quoted_string(bytes, i, b'"');
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'/' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'/' {
                let end = skip_line_comment(bytes, i);
                comments.push(source[i + 2..end].to_string());
                mask_range(&mut masked, bytes, i, end);
                i = end;
                continue;
            }

            if bytes[i + 1] == b'*' {
                let end = skip_block_comment(bytes, i, language.uses_nested_block_comments());
                let content_end = if end >= i + 4 { end - 2 } else { end };
                comments.push(source[i + 2..content_end].to_string());
                mask_range(&mut masked, bytes, i, end);
                i = end;
                continue;
            }
        }

        i += 1;
    }

    LexedSource {
        comments,
        masked_code: String::from_utf8(masked).unwrap_or_else(|_| source.to_string()),
    }
}

fn lex_php_source(source: &str) -> LexedSource {
    let bytes = source.as_bytes();
    let mut masked = bytes.to_vec();
    let mut comments = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if is_csharp_verbatim_string_start(bytes, i) {
            let end = skip_csharp_verbatim_string(bytes, i);
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'`' {
            let end = skip_backtick_string(bytes, i);
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if uses_single_quote_strings(SourceLanguage::Php) && (bytes[i] == b'\'' || bytes[i] == b'"')
        {
            let end = skip_quoted_string(bytes, i, bytes[i]);
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'#' {
            if matches!(bytes.get(i + 1), Some(b'[')) {
                i += 1;
                continue;
            }

            let end = skip_line_comment(bytes, i);
            comments.push(source[i + 1..end].to_string());
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'/' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'/' {
                let end = skip_line_comment(bytes, i);
                comments.push(source[i + 2..end].to_string());
                mask_range(&mut masked, bytes, i, end);
                i = end;
                continue;
            }

            if bytes[i + 1] == b'*' {
                let end = skip_block_comment(bytes, i, false);
                let content_end = if end >= i + 4 { end - 2 } else { end };
                comments.push(source[i + 2..content_end].to_string());
                mask_range(&mut masked, bytes, i, end);
                i = end;
                continue;
            }
        }

        i += 1;
    }

    LexedSource {
        comments,
        masked_code: String::from_utf8(masked).unwrap_or_else(|_| source.to_string()),
    }
}

fn lex_lua_family_source(source: &str) -> LexedSource {
    let bytes = source.as_bytes();
    let mut masked = bytes.to_vec();
    let mut comments = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if let Some((_, _, end)) = skip_lua_long_bracket(bytes, i) {
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'\'' || bytes[i] == b'"' {
            let end = skip_quoted_string(bytes, i, bytes[i]);
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            if let Some((content_start, content_end, end)) = skip_lua_long_bracket(bytes, i + 2) {
                comments.push(source[content_start..content_end].to_string());
                mask_range(&mut masked, bytes, i, end);
                i = end;
                continue;
            }

            let end = skip_line_comment(bytes, i);
            comments.push(source[i + 2..end].to_string());
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        i += 1;
    }

    LexedSource {
        comments,
        masked_code: String::from_utf8(masked).unwrap_or_else(|_| source.to_string()),
    }
}

fn lex_ruby_source(source: &str) -> LexedSource {
    let bytes = source.as_bytes();
    let mut masked = bytes.to_vec();
    let mut comments = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if let Some((content_start, content_end, end)) = skip_ruby_block_comment(source, bytes, i) {
            comments.push(source[content_start..content_end].to_string());
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if let Some(end) = skip_ruby_heredoc(source, bytes, i) {
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'`' || bytes[i] == b'\'' || bytes[i] == b'"' {
            let end = skip_quoted_string(bytes, i, bytes[i]);
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'#' {
            let end = skip_line_comment(bytes, i);
            comments.push(source[i + 1..end].to_string());
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        i += 1;
    }

    LexedSource {
        comments,
        masked_code: String::from_utf8(masked).unwrap_or_else(|_| source.to_string()),
    }
}

fn lex_python_source(source: &str) -> LexedSource {
    let bytes = source.as_bytes();
    let mut masked = bytes.to_vec();
    let mut comments = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if let Some(end) = skip_python_triple_quoted_string(bytes, i) {
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'\'' || bytes[i] == b'"' {
            let end = skip_quoted_string(bytes, i, bytes[i]);
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        if bytes[i] == b'#' {
            let end = skip_line_comment(bytes, i);
            comments.push(source[i + 1..end].to_string());
            mask_range(&mut masked, bytes, i, end);
            i = end;
            continue;
        }

        i += 1;
    }

    LexedSource {
        comments,
        masked_code: String::from_utf8(masked).unwrap_or_else(|_| source.to_string()),
    }
}

fn uses_backtick_strings(language: SourceLanguage) -> bool {
    matches!(
        language,
        SourceLanguage::TypeScript
            | SourceLanguage::Tsx
            | SourceLanguage::JavaScript
            | SourceLanguage::Jsx
            | SourceLanguage::Go
    )
}

fn uses_single_quote_strings(language: SourceLanguage) -> bool {
    !matches!(language, SourceLanguage::Rust)
}

fn mask_range(masked: &mut [u8], original: &[u8], start: usize, end: usize) {
    for index in start..end.min(masked.len()) {
        masked[index] = match original[index] {
            b'\n' | b'\r' | b'\t' | b' ' => original[index],
            _ => b' ',
        };
    }
}

fn skip_line_comment(bytes: &[u8], start: usize) -> usize {
    let mut index = start;
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    index
}

fn skip_block_comment(bytes: &[u8], start: usize, allow_nested: bool) -> usize {
    let mut index = start + 2;
    let mut depth = 1usize;

    while index + 1 < bytes.len() {
        if allow_nested && bytes[index] == b'/' && bytes[index + 1] == b'*' {
            depth += 1;
            index += 2;
            continue;
        }

        if bytes[index] == b'*' && bytes[index + 1] == b'/' {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return index;
            }
            continue;
        }

        index += 1;
    }

    bytes.len()
}

fn skip_quoted_string(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut index = start + 1;

    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
            continue;
        }

        if bytes[index] == quote {
            return index + 1;
        }

        index += 1;
    }

    bytes.len()
}

fn skip_swift_string(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start;
    let mut hashes = 0usize;

    while index < bytes.len() && bytes[index] == b'#' {
        hashes += 1;
        index += 1;
    }

    if index >= bytes.len() || bytes[index] != b'"' {
        return None;
    }

    let multiline = matches!(bytes.get(index..index + 3), Some(prefix) if prefix == b"\"\"\"");
    index += if multiline { 3 } else { 1 };

    while index < bytes.len() {
        if !multiline && bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
            continue;
        }

        if multiline {
            if !matches!(bytes.get(index..index + 3), Some(prefix) if prefix == b"\"\"\"") {
                index += 1;
                continue;
            }

            let end = index + 3;
            if matches_hash_suffix(bytes, end, hashes) {
                return Some((end + hashes).min(bytes.len()));
            }

            index += 1;
            continue;
        }

        if bytes[index] == b'"' && matches_hash_suffix(bytes, index + 1, hashes) {
            return Some((index + 1 + hashes).min(bytes.len()));
        }

        index += 1;
    }

    Some(bytes.len())
}

fn skip_backtick_string(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 1;

    while index < bytes.len() {
        if bytes[index] == b'`' {
            return index + 1;
        }
        index += 1;
    }

    bytes.len()
}

fn skip_python_triple_quoted_string(bytes: &[u8], start: usize) -> Option<usize> {
    if start + 2 >= bytes.len() {
        return None;
    }

    let quote = bytes[start];
    if (quote == b'\'' || quote == b'"') && bytes[start + 1] == quote && bytes[start + 2] == quote {
        let mut index = start + 3;
        while index + 2 < bytes.len() {
            if bytes[index] == quote && bytes[index + 1] == quote && bytes[index + 2] == quote {
                return Some(index + 3);
            }
            index += 1;
        }
        return Some(bytes.len());
    }

    None
}

fn skip_ruby_block_comment(
    source: &str,
    bytes: &[u8],
    start: usize,
) -> Option<(usize, usize, usize)> {
    if !is_line_start(bytes, start)
        || !matches!(bytes.get(start..start + 6), Some(prefix) if prefix == b"=begin")
    {
        return None;
    }

    let content_start = skip_line_comment(bytes, start);
    let mut line_start = content_start;
    while line_start < bytes.len() {
        if bytes[line_start] == b'\n' {
            line_start += 1;
        }

        let line_end = skip_line_comment(bytes, line_start);
        let line = source.get(line_start..line_end)?.trim_start();
        if line.starts_with("=end") {
            return Some((content_start, line_start, line_end));
        }

        line_start = line_end;
    }

    Some((content_start, bytes.len(), bytes.len()))
}

fn skip_ruby_heredoc(source: &str, bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'<') || bytes.get(start + 1) != Some(&b'<') {
        return None;
    }
    if start > 0 && bytes[start - 1] == b'<' {
        return None;
    }

    let mut index = start + 2;
    let allow_indent = matches!(bytes.get(index), Some(b'-' | b'~'));
    if allow_indent {
        index += 1;
    }
    if index >= bytes.len() {
        return None;
    }

    let mut delimiter = String::new();
    if matches!(bytes[index], b'\'' | b'"' | b'`') {
        let quote = bytes[index];
        index += 1;
        let content_start = index;
        while index < bytes.len() && bytes[index] != quote {
            index += 1;
        }
        if index >= bytes.len() {
            return Some(bytes.len());
        }
        delimiter.push_str(source.get(content_start..index)?);
        index += 1;
    } else {
        let content_start = index;
        while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
        {
            index += 1;
        }
        if index == content_start {
            return None;
        }
        delimiter.push_str(source.get(content_start..index)?);
    }

    if delimiter.is_empty()
        || !delimiter
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
    {
        return None;
    }

    while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
        index += 1;
    }
    if index < bytes.len() && bytes[index] != b'\n' {
        return None;
    }
    if index < bytes.len() {
        index += 1;
    }

    while index < bytes.len() {
        let line_end = skip_line_comment(bytes, index);
        let line = source.get(index..line_end)?;
        let trimmed = if allow_indent {
            line.trim_start_matches([' ', '\t'])
        } else {
            line
        };
        if trimmed == delimiter {
            return Some(line_end);
        }

        index = if line_end < bytes.len() {
            line_end + 1
        } else {
            line_end
        };
    }

    Some(bytes.len())
}

fn skip_lua_long_bracket(bytes: &[u8], start: usize) -> Option<(usize, usize, usize)> {
    if bytes.get(start) != Some(&b'[') {
        return None;
    }

    let mut index = start + 1;
    let mut equals_count = 0usize;
    while index < bytes.len() && bytes[index] == b'=' {
        equals_count += 1;
        index += 1;
    }

    if bytes.get(index) != Some(&b'[') {
        return None;
    }

    let content_start = index + 1;
    let mut cursor = content_start;
    while cursor < bytes.len() {
        if bytes[cursor] != b']' {
            cursor += 1;
            continue;
        }

        let mut end = cursor + 1;
        let mut matched = 0usize;
        while matched < equals_count && end < bytes.len() && bytes[end] == b'=' {
            matched += 1;
            end += 1;
        }

        if matched == equals_count && bytes.get(end) == Some(&b']') {
            return Some((content_start, cursor, end + 1));
        }

        cursor += 1;
    }

    Some((content_start, bytes.len(), bytes.len()))
}

fn matches_hash_suffix(bytes: &[u8], start: usize, hashes: usize) -> bool {
    start + hashes <= bytes.len()
        && bytes[start..start + hashes]
            .iter()
            .all(|byte| *byte == b'#')
}

fn is_line_start(bytes: &[u8], index: usize) -> bool {
    index == 0 || matches!(bytes.get(index - 1), Some(b'\n'))
}

fn skip_rust_raw_string(bytes: &[u8], start: usize) -> Option<usize> {
    if !matches!(bytes[start], b'r' | b'b') {
        return None;
    }

    if start > 0 && is_identifier_continue(bytes[start - 1]) {
        return None;
    }

    let mut index = start;
    if bytes[index] == b'b' {
        index += 1;
        if index >= bytes.len() || bytes[index] != b'r' {
            return None;
        }
    }

    if bytes[index] != b'r' {
        return None;
    }
    index += 1;

    let mut hashes = 0usize;
    while index < bytes.len() && bytes[index] == b'#' {
        hashes += 1;
        index += 1;
    }

    if index >= bytes.len() || bytes[index] != b'"' {
        return None;
    }
    index += 1;

    while index < bytes.len() {
        if bytes[index] == b'"' {
            let mut matched = true;
            for offset in 0..hashes {
                if index + 1 + offset >= bytes.len() || bytes[index + 1 + offset] != b'#' {
                    matched = false;
                    break;
                }
            }
            if matched {
                return Some((index + 1 + hashes).min(bytes.len()));
            }
        }
        index += 1;
    }

    Some(bytes.len())
}

fn is_csharp_verbatim_string_start(bytes: &[u8], start: usize) -> bool {
    matches!(
        bytes.get(start..start + 2),
        Some(prefix) if prefix == b"@\""
    ) || matches!(bytes.get(start..start + 3), Some(prefix) if prefix == b"$@\"" || prefix == b"@$\"")
}

fn skip_csharp_verbatim_string(bytes: &[u8], start: usize) -> usize {
    let quote_start = if matches!(bytes.get(start..start + 3), Some(prefix) if prefix == b"$@\"" || prefix == b"@$\"")
    {
        start + 3
    } else {
        start + 2
    };

    let mut index = quote_start;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            if index + 1 < bytes.len() && bytes[index + 1] == b'"' {
                index += 2;
                continue;
            }
            return index + 1;
        }
        index += 1;
    }

    bytes.len()
}

fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn extract_rust_public_api(source: &str) -> Option<Vec<String>> {
    let parsed = syn::parse_file(source).ok()?;
    let mut signatures = Vec::new();
    collect_rust_items(&parsed.items, "", &mut signatures);
    signatures.sort();
    signatures.dedup();
    Some(signatures)
}

fn collect_rust_items(items: &[Item], module_prefix: &str, signatures: &mut Vec<String>) {
    for item in items {
        match item {
            Item::Fn(item_fn) if is_public(&item_fn.vis) => {
                let sig = format_rust_signature(&item_fn.sig);
                signatures.push(format!("rust:fn {module_prefix}{sig}"));
            }
            Item::Struct(item_struct) if is_public(&item_struct.vis) => {
                let sig = format!(
                    "{}{}",
                    item_struct.ident,
                    normalize_signature(&item_struct.generics.to_token_stream().to_string())
                );
                signatures.push(format!("rust:struct {module_prefix}{sig}"));
            }
            Item::Enum(item_enum) if is_public(&item_enum.vis) => {
                let sig = format!(
                    "{}{}",
                    item_enum.ident,
                    normalize_signature(&item_enum.generics.to_token_stream().to_string())
                );
                signatures.push(format!("rust:enum {module_prefix}{sig}"));
            }
            Item::Trait(item_trait) if is_public(&item_trait.vis) => {
                let trait_name = format!("{}{name}", module_prefix, name = item_trait.ident);
                signatures.push(format!("rust:trait {trait_name}"));
                for trait_item in &item_trait.items {
                    if let TraitItem::Fn(method) = trait_item {
                        let sig = format_rust_signature(&method.sig);
                        signatures.push(format!("rust:trait-fn {trait_name}::{sig}"));
                    }
                }
            }
            Item::Type(item_type) if is_public(&item_type.vis) => {
                let ty = normalize_signature(&item_type.ty.to_token_stream().to_string());
                signatures.push(format!("rust:type {module_prefix}{}={ty}", item_type.ident));
            }
            Item::Const(item_const) if is_public(&item_const.vis) => {
                let ty = normalize_signature(&item_const.ty.to_token_stream().to_string());
                signatures.push(format!(
                    "rust:const {module_prefix}{}:{ty}",
                    item_const.ident
                ));
            }
            Item::Static(item_static) if is_public(&item_static.vis) => {
                let ty = normalize_signature(&item_static.ty.to_token_stream().to_string());
                signatures.push(format!(
                    "rust:static {module_prefix}{}:{ty}",
                    item_static.ident
                ));
            }
            Item::Impl(item_impl) => {
                let impl_scope = item_impl
                    .trait_
                    .as_ref()
                    .map(|(_, path, _)| normalize_signature(&path.to_token_stream().to_string()))
                    .unwrap_or_else(|| {
                        normalize_signature(&item_impl.self_ty.to_token_stream().to_string())
                    });

                for impl_item in &item_impl.items {
                    if let ImplItem::Fn(method) = impl_item {
                        if !is_public(&method.vis) {
                            continue;
                        }

                        let sig = format_rust_signature(&method.sig);
                        signatures.push(format!("rust:impl-fn {module_prefix}{impl_scope}::{sig}"));
                    }
                }
            }
            Item::Mod(item_mod) if is_public(&item_mod.vis) => {
                signatures.push(format!("rust:mod {module_prefix}{}", item_mod.ident));
                if let Some((_, nested_items)) = &item_mod.content {
                    let nested_prefix = format!("{}{name}::", module_prefix, name = item_mod.ident);
                    collect_rust_items(nested_items, &nested_prefix, signatures);
                }
            }
            _ => {}
        }
    }
}

fn is_public(visibility: &Visibility) -> bool {
    matches!(visibility, Visibility::Public(_))
}

fn format_rust_signature(signature: &Signature) -> String {
    let qualifiers = format_rust_qualifiers(signature);
    let generics = format_rust_generics(signature);
    let params = signature
        .inputs
        .iter()
        .map(format_rust_fn_arg)
        .collect::<Vec<_>>()
        .join(", ");
    let output = format_rust_return_type(&signature.output);
    let where_clause = signature
        .generics
        .where_clause
        .as_ref()
        .map(|clause| {
            let normalized = normalize_signature(&clause.to_token_stream().to_string());
            format!(" {}", normalized.trim_end_matches(','))
        })
        .unwrap_or_default();

    format!(
        "{qualifiers}{}{generics}({params}){output}{where_clause}",
        signature.ident
    )
}

fn format_rust_qualifiers(signature: &Signature) -> String {
    let mut qualifiers = Vec::new();
    if signature.constness.is_some() {
        qualifiers.push("const".to_string());
    }
    if signature.asyncness.is_some() {
        qualifiers.push("async".to_string());
    }
    if signature.unsafety.is_some() {
        qualifiers.push("unsafe".to_string());
    }
    if let Some(abi) = &signature.abi {
        qualifiers.push(normalize_signature(&abi.to_token_stream().to_string()));
    }

    if qualifiers.is_empty() {
        String::new()
    } else {
        format!("{} ", qualifiers.join(" "))
    }
}

fn format_rust_generics(signature: &Signature) -> String {
    let params = normalize_signature(&signature.generics.params.to_token_stream().to_string());
    if params.is_empty() {
        String::new()
    } else {
        format!("<{params}>")
    }
}

fn format_rust_fn_arg(argument: &FnArg) -> String {
    match argument {
        FnArg::Receiver(receiver) => {
            if receiver.colon_token.is_some() {
                let mut rendered = String::new();
                if receiver.mutability.is_some() {
                    rendered.push_str("mut ");
                }
                rendered.push_str("self: ");
                rendered.push_str(&normalize_signature(
                    &receiver.ty.to_token_stream().to_string(),
                ));
                return rendered;
            }

            let mut rendered = String::new();
            if let Some((_, lifetime)) = &receiver.reference {
                rendered.push('&');
                if let Some(lifetime) = lifetime {
                    rendered.push_str(&lifetime.to_token_stream().to_string());
                    rendered.push(' ');
                }
            }
            if receiver.mutability.is_some() {
                rendered.push_str("mut ");
            }
            rendered.push_str("self");
            rendered
        }
        FnArg::Typed(argument) => {
            let pattern = normalize_signature(&argument.pat.to_token_stream().to_string());
            let ty = normalize_signature(&argument.ty.to_token_stream().to_string());
            format!("{pattern}: {ty}")
        }
    }
}

fn format_rust_return_type(output: &ReturnType) -> String {
    match output {
        ReturnType::Default => String::new(),
        ReturnType::Type(_, ty) => {
            format!(
                " -> {}",
                normalize_signature(&ty.to_token_stream().to_string())
            )
        }
    }
}

fn normalize_signature(raw: &str) -> String {
    let mut normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    for (from, to) in [
        (" (", "("),
        ("( ", "("),
        (" )", ")"),
        (" ,", ","),
        (" ;", ";"),
        (" :", ":"),
        (" ::", "::"),
        (" < ", "<"),
        ("< ", "<"),
        (" >", ">"),
        (" = ", "="),
        ("& mut ", "&mut "),
        ("& ", "&"),
        (" & ", "&"),
    ] {
        normalized = normalized.replace(from, to);
    }
    normalized
}

fn is_comment_line(line: &str, extension: &str) -> bool {
    match extension {
        "py" => line.starts_with('#'),
        "php" => line.starts_with('#') || line.starts_with("//") || line.starts_with("/*"),
        "rb" => line.starts_with('#') || line.starts_with("=begin"),
        "lua" | "luau" => line.starts_with("--"),
        _ => line.starts_with("//") || line.starts_with("/*") || line.starts_with('*'),
    }
}

fn exclusion_decision(
    root: &Path,
    scan_policy: &ScanPolicy,
    entry: &DirEntry,
) -> Option<ExcludedPathDecision> {
    if entry.depth() == 0 {
        return None;
    }

    let rel = relative_display_path(root, entry.path());
    if let Some(rule) = scan_policy.matching_excluded_path(&rel) {
        return Some(ExcludedPathDecision {
            path: rel,
            kind: entry_path_kind(entry),
            reason: "scan_policy_exclude_path".to_string(),
            rule: Some(rule),
        });
    }

    if entry.file_type().is_dir() {
        return default_ignored_directory_name(entry.file_name().to_str()?).map(|rule| {
            ExcludedPathDecision {
                path: rel,
                kind: ExcludedPathKind::Directory,
                reason: "default_ignored_directory".to_string(),
                rule: Some(rule.to_string()),
            }
        });
    }

    None
}

fn entry_path_kind(entry: &DirEntry) -> ExcludedPathKind {
    if entry.file_type().is_dir() {
        ExcludedPathKind::Directory
    } else {
        ExcludedPathKind::File
    }
}

fn default_ignored_directory_name(name: &str) -> Option<&'static str> {
    match name {
        ".git" => Some(".git"),
        "node_modules" => Some("node_modules"),
        "dist" => Some("dist"),
        "build" => Some("build"),
        "target" => Some("target"),
        "vendor" => Some("vendor"),
        ".next" => Some(".next"),
        "tests" => Some("tests"),
        "examples" => Some("examples"),
        "benches" => Some("benches"),
        _ => None,
    }
}

fn relative_display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

#[allow(dead_code)]
fn normalize_path(path: &Path) -> PathBuf {
    path.components().collect()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::mvs::manifest::ScanPolicy;

    use super::{crawl_codebase, ExcludedPathKind};

    struct TempWorkspace {
        path: PathBuf,
    }

    impl TempWorkspace {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let index = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mvs-crawler-test-{}-{}-{}",
                std::process::id(),
                nanos,
                index
            ));
            fs::create_dir_all(&path).expect("failed to create temp workspace");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    struct ParserAdapterCase<'a> {
        file_name: &'a str,
        source: &'a str,
        expected_feature: &'a str,
        expected_protocol: &'a str,
        expected_public_api: &'a [&'a str],
        rejected_public_api_fragments: &'a [&'a str],
    }

    fn assert_parser_adapter_case(case: ParserAdapterCase<'_>) {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");
        fs::write(src.join(case.file_name), case.source).expect("failed to write parser fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.contains(case.expected_feature));
        assert!(report.protocol_tags.contains(case.expected_protocol));

        for signature in case.expected_public_api {
            assert!(
                report
                    .public_api
                    .iter()
                    .any(|entry| entry.signature == *signature),
                "missing public API signature {signature} for {}",
                case.file_name
            );
        }

        for fragment in case.rejected_public_api_fragments {
            assert!(
                !report
                    .public_api
                    .iter()
                    .any(|entry| entry.signature.contains(fragment)),
                "unexpected public API fragment {fragment} for {}",
                case.file_name
            );
        }
    }

    #[test]
    fn crawls_tags_and_public_api() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.ts"),
            r#"
            // @mvs-feature("offline_storage")
            // @mvs-protocol("auth-api-v1")
            export function login(username: string): Promise<string> {
              return Promise.resolve(username)
            }
            export interface Session {
              token: string
            }
        "#,
        )
        .expect("failed to write ts fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.contains("offline_storage"));
        assert!(report.protocol_tags.contains("auth-api-v1"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("login")));
    }

    #[test]
    fn rust_ast_extraction_tracks_pub_signatures() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("lib.rs"),
            r#"
            /// @mvs-feature("core_runtime")
            /// @mvs-protocol("rust_host_surface")
            pub fn handshake(version: u32) -> bool { version > 0 }

            pub struct HostAdapter;

            impl HostAdapter {
                pub fn connect(&self, target: &str) -> bool { !target.is_empty() }
                fn private_method(&self) {}
            }
        "#,
        )
        .expect("failed to write rust fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "rust:fn handshake(version: u32) -> bool"));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "rust:impl-fn HostAdapter::connect(&self, target: &str) -> bool"
        }));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("private_method")));
    }

    #[test]
    fn rust_ast_extraction_preserves_generics_and_where_clauses_in_canonical_form() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("lib.rs"),
            r#"
            pub async fn load<'a, T>(value: &'a T) -> &'a T
            where
                T: Clone,
            {
                value
            }
        "#,
        )
        .expect("failed to write rust fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "rust:fn async load<'a, T>(value: &'a T) -> &'a T where T: Clone"
        }));
    }

    #[test]
    fn rust_export_following_public_modules_expands_rooted_lib_rs_across_public_modules() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(src.join("facade")).expect("failed to create facade dir");

        fs::write(
            src.join("lib.rs"),
            r#"
            pub fn handshake(version: u32) -> bool { version > 0 }

            pub mod api;
            mod internal;

            pub mod facade {
                pub mod http;
            }
        "#,
        )
        .expect("failed to write rust root fixture");

        fs::write(
            src.join("api.rs"),
            r#"
            pub struct Session;
        "#,
        )
        .expect("failed to write rust api fixture");

        fs::write(
            src.join("internal.rs"),
            r#"
            pub struct Hidden;
        "#,
        )
        .expect("failed to write rust internal fixture");

        fs::write(
            src.join("facade/http.rs"),
            r#"
            pub fn respond(status: u16) -> bool { status > 0 }
        "#,
        )
        .expect("failed to write rust nested fixture");

        let default_report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/lib.rs".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(default_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "rust:fn handshake(version: u32) -> bool"));
        assert!(!default_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "rust:struct Session"));
        assert!(!default_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "rust:fn respond(status: u16) -> bool"));

        let followed_report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/lib.rs".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::PublicModules,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(followed_report.public_api.iter().any(|entry| {
            entry.file == "src/api.rs" && entry.signature == "rust:struct Session"
        }));
        assert!(followed_report.public_api.iter().any(|entry| {
            entry.file == "src/facade/http.rs"
                && entry.signature == "rust:fn respond(status: u16) -> bool"
        }));
        assert!(!followed_report
            .public_api
            .iter()
            .any(|entry| entry.file == "src/internal.rs"));
    }

    #[test]
    fn rust_export_following_public_modules_resolves_direct_pub_use_from_private_modules() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("lib.rs"),
            r#"
            pub use internal::{Hidden as Visible, connect as open};

            mod internal;
        "#,
        )
        .expect("failed to write rust root fixture");

        fs::write(
            src.join("internal.rs"),
            r#"
            pub struct Hidden;

            impl Hidden {
                pub fn ping(&self) -> bool { true }
            }

            pub fn connect(target: u32) -> bool { target > 0 }

            pub struct Other;
        "#,
        )
        .expect("failed to write rust internal fixture");

        let report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/lib.rs".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::PublicModules,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.file == "src/lib.rs" && entry.signature == "rust:struct Visible"));
        assert!(report.public_api.iter().any(|entry| {
            entry.file == "src/lib.rs"
                && entry.signature == "rust:impl-fn Visible::ping(&self) -> bool"
        }));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.file == "src/lib.rs"
                && entry.signature == "rust:fn open(target: u32) -> bool"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("Hidden")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("Other")));
    }

    #[test]
    fn rust_export_following_public_modules_resolves_chained_pub_use_aliases_and_globs() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("lib.rs"),
            r#"
            pub use facade::{PublicSession as Session, open as connect};
            pub use facade::*;

            pub mod facade;
            mod internal;
        "#,
        )
        .expect("failed to write rust root fixture");

        fs::write(
            src.join("facade.rs"),
            r#"
            pub use crate::internal::{Hidden as PublicSession, start as open};
        "#,
        )
        .expect("failed to write rust facade fixture");

        fs::write(
            src.join("internal.rs"),
            r#"
            pub struct Hidden;

            impl Hidden {
                pub fn ping(&self) -> bool { true }
            }

            pub fn start(target: u32) -> bool { target > 0 }
        "#,
        )
        .expect("failed to write rust internal fixture");

        let report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/lib.rs".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::PublicModules,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.file == "src/lib.rs" && entry.signature == "rust:struct Session"));
        assert!(report.public_api.iter().any(|entry| {
            entry.file == "src/lib.rs"
                && entry.signature == "rust:impl-fn Session::ping(&self) -> bool"
        }));
        assert!(report.public_api.iter().any(|entry| {
            entry.file == "src/lib.rs" && entry.signature == "rust:fn connect(target: u32) -> bool"
        }));
        assert!(report.public_api.iter().any(|entry| {
            entry.file == "src/lib.rs" && entry.signature == "rust:struct PublicSession"
        }));
        assert!(report.public_api.iter().any(|entry| {
            entry.file == "src/lib.rs" && entry.signature == "rust:fn open(target: u32) -> bool"
        }));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("Hidden")));
    }

    #[test]
    fn rust_export_following_public_modules_resolves_explicit_workspace_member_reexports() {
        let workspace = TempWorkspace::new();
        let app_src = workspace.path().join("app/src");
        let shared_src = workspace.path().join("shared/src");
        fs::create_dir_all(&app_src).expect("failed to create app/src");
        fs::create_dir_all(&shared_src).expect("failed to create shared/src");

        fs::write(
            workspace.path().join("app/Cargo.toml"),
            r#"
            [package]
            name = "app"
        "#,
        )
        .expect("failed to write app Cargo.toml");

        fs::write(
            workspace.path().join("shared/Cargo.toml"),
            r#"
            [package]
            name = "shared-contract"
        "#,
        )
        .expect("failed to write shared Cargo.toml");

        fs::write(
            app_src.join("lib.rs"),
            r#"
            pub use shared_contract::{Hidden as Session, connect as open};
        "#,
        )
        .expect("failed to write app rust root fixture");

        fs::write(
            shared_src.join("lib.rs"),
            r#"
            pub struct Hidden;

            impl Hidden {
                pub fn ping(&self) -> bool { true }
            }

            pub fn connect(target: u32) -> bool { target > 0 }
        "#,
        )
        .expect("failed to write shared rust fixture");

        let default_report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["app/src/lib.rs".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::PublicModules,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(!default_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "rust:struct Session"));
        assert!(!default_report
            .public_api
            .iter()
            .any(|entry| { entry.signature == "rust:impl-fn Session::ping(&self) -> bool" }));
        assert!(!default_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "rust:fn open(target: u32) -> bool"));

        let followed_report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["app/src/lib.rs".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::PublicModules,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: vec!["shared".to_string()],
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(followed_report.public_api.iter().any(|entry| {
            entry.file == "app/src/lib.rs" && entry.signature == "rust:struct Session"
        }));
        assert!(followed_report.public_api.iter().any(|entry| {
            entry.file == "app/src/lib.rs"
                && entry.signature == "rust:impl-fn Session::ping(&self) -> bool"
        }));
        assert!(followed_report.public_api.iter().any(|entry| {
            entry.file == "app/src/lib.rs" && entry.signature == "rust:fn open(target: u32) -> bool"
        }));
        assert!(!followed_report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("shared_contract::")));
    }

    #[test]
    fn rust_export_following_public_modules_resolves_chained_workspace_member_reexports() {
        let workspace = TempWorkspace::new();
        let app_src = workspace.path().join("app/src");
        let sdk_src = workspace.path().join("sdk/src");
        let shared_contract = workspace.path().join("shared/contract");
        fs::create_dir_all(&app_src).expect("failed to create app/src");
        fs::create_dir_all(&sdk_src).expect("failed to create sdk/src");
        fs::create_dir_all(&shared_contract).expect("failed to create shared/contract");

        fs::write(
            workspace.path().join("app/Cargo.toml"),
            r#"
            [package]
            name = "app"
        "#,
        )
        .expect("failed to write app Cargo.toml");

        fs::write(
            workspace.path().join("sdk/Cargo.toml"),
            r#"
            [package]
            name = "sdk"
        "#,
        )
        .expect("failed to write sdk Cargo.toml");

        fs::write(
            workspace.path().join("shared/Cargo.toml"),
            r#"
            [package]
            name = "shared-contract"

            [lib]
            path = "contract/mod.rs"
        "#,
        )
        .expect("failed to write shared Cargo.toml");

        fs::write(
            app_src.join("lib.rs"),
            r#"
            pub use sdk::{PublicSession, open};
        "#,
        )
        .expect("failed to write app rust root fixture");

        fs::write(
            sdk_src.join("lib.rs"),
            r#"
            pub use shared_contract::{Hidden as PublicSession, connect as open};
        "#,
        )
        .expect("failed to write sdk rust root fixture");

        fs::write(
            shared_contract.join("mod.rs"),
            r#"
            pub struct Hidden;

            impl Hidden {
                pub fn ping(&self) -> bool { true }
            }

            pub fn connect(target: u32) -> bool { target > 0 }
        "#,
        )
        .expect("failed to write shared rust fixture");

        let report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["app/src/lib.rs".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::PublicModules,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: vec!["sdk".to_string(), "shared".to_string()],
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(report.public_api.iter().any(|entry| {
            entry.file == "app/src/lib.rs" && entry.signature == "rust:struct PublicSession"
        }));
        assert!(report.public_api.iter().any(|entry| {
            entry.file == "app/src/lib.rs"
                && entry.signature == "rust:impl-fn PublicSession::ping(&self) -> bool"
        }));
        assert!(report.public_api.iter().any(|entry| {
            entry.file == "app/src/lib.rs" && entry.signature == "rust:fn open(target: u32) -> bool"
        }));
    }

    #[test]
    fn excludes_tests_directory_by_default() {
        let workspace = TempWorkspace::new();
        let tests_dir = workspace.path().join("tests");
        fs::create_dir_all(&tests_dir).expect("failed to create tests dir");
        fs::write(
            tests_dir.join("api.ts"),
            "// @mvs-feature(\"test_only\")\nexport function helper() {}\n",
        )
        .expect("failed to write tests file");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.is_empty());
        assert!(report.public_api.is_empty());
        assert!(report.excluded_paths.iter().any(|decision| {
            decision.path == "tests"
                && decision.kind == ExcludedPathKind::Directory
                && decision.reason == "default_ignored_directory"
                && decision.rule.as_deref() == Some("tests")
        }));
    }

    #[test]
    fn ignores_tags_inside_rust_raw_strings() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("lib.rs"),
            r##"
            const EXAMPLE: &str = r#"
            // @mvs-feature("fake_feature")
            // @mvs-protocol("fake_protocol")
            "#;

            /// @mvs-feature("real_feature")
            /// @mvs-protocol("real_protocol")
            pub fn handshake(version: u32) -> bool { version > 0 }
        "##,
        )
        .expect("failed to write rust fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.contains("real_feature"));
        assert!(report.protocol_tags.contains("real_protocol"));
        assert!(!report.feature_tags.contains("fake_feature"));
        assert!(!report.protocol_tags.contains("fake_protocol"));
    }

    #[test]
    fn captures_tags_from_block_comments() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("billing.ts"),
            r#"
            /*
             * @mvs-feature("billing")
             * @mvs-protocol("billing-api-v1")
             */
            export function checkout(amount: number): number {
              return amount
            }
        "#,
        )
        .expect("failed to write ts fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.contains("billing"));
        assert!(report.protocol_tags.contains("billing-api-v1"));
    }

    #[test]
    fn tree_sitter_captures_named_exports_and_reexports() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.ts"),
            r#"
            const login = (username: string): string => username;
            function logout(): void {}

            export {
              login,
              logout as signOut,
            };

            export { login as authenticate } from "./auth";
            export * as authApi from "./auth";
        "#,
        )
        .expect("failed to write ts fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| { entry.signature == "ts/js:export { login, logout as signOut }" }));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "ts/js:export { login as authenticate } from \"./auth\""
        }));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:export * as authApi from \"./auth\""));
    }

    #[test]
    fn ts_export_following_relative_only_resolves_barrel_reexports() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("auth.ts"),
            r#"
            export function login(username: string): string {
              return username;
            }

            export interface Session {
              token: string;
            }
        "#,
        )
        .expect("failed to write auth fixture");

        fs::write(
            src.join("index.ts"),
            r#"
            export { login as authenticate, Session as ActiveSession } from "./auth";
            export * from "./auth";
        "#,
        )
        .expect("failed to write index fixture");

        let default_report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/index.ts".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(default_report.public_api.iter().any(|entry| {
            entry.signature
                == "ts/js:export { login as authenticate, Session as ActiveSession } from \"./auth\""
        }));
        assert!(default_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:export * from \"./auth\""));
        assert!(!default_report.public_api.iter().any(
            |entry| entry.signature == "ts/js:function authenticate(username: string): string"
        ));

        let followed_report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/index.ts".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::RelativeOnly,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(followed_report.public_api.iter().any(
            |entry| entry.signature == "ts/js:function authenticate(username: string): string"
        ));
        assert!(followed_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:function login(username: string): string"));
        assert!(followed_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:interface ActiveSession"));
        assert!(followed_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:interface Session"));
        assert!(!followed_report.public_api.iter().any(|entry| {
            entry.signature
                == "ts/js:export { login as authenticate, Session as ActiveSession } from \"./auth\""
        }));
        assert!(!followed_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:export * from \"./auth\""));
    }

    #[test]
    fn ts_export_following_workspace_only_resolves_package_exports_and_tsconfig_paths() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            workspace.path().join("package.json"),
            r#"
            {
              "name": "@demo/sdk",
              "exports": {
                "./auth": "./src/auth.ts"
              }
            }
        "#,
        )
        .expect("failed to write package.json");

        fs::write(
            workspace.path().join("tsconfig.json"),
            r#"
            {
              "compilerOptions": {
                "baseUrl": ".",
                "paths": {
                  "@/*": ["src/*"]
                }
              }
            }
        "#,
        )
        .expect("failed to write tsconfig.json");

        fs::write(
            src.join("auth.ts"),
            r#"
            export function login(username: string): string {
              return username;
            }
        "#,
        )
        .expect("failed to write auth fixture");

        fs::write(
            src.join("session.ts"),
            r#"
            export interface Session {
              token: string;
            }
        "#,
        )
        .expect("failed to write session fixture");

        fs::write(
            src.join("index.ts"),
            r#"
            export { login as authenticate } from "@demo/sdk/auth";
            export * from "@/session";
        "#,
        )
        .expect("failed to write index fixture");

        let default_report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/index.ts".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::RelativeOnly,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(default_report.public_api.iter().any(|entry| {
            entry.signature == "ts/js:export { login as authenticate } from \"@demo/sdk/auth\""
        }));
        assert!(default_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:export * from \"@/session\""));

        let workspace_report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/index.ts".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::WorkspaceOnly,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(workspace_report.public_api.iter().any(
            |entry| entry.signature == "ts/js:function authenticate(username: string): string"
        ));
        assert!(workspace_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:interface Session"));
        assert!(!workspace_report.public_api.iter().any(|entry| {
            entry.signature == "ts/js:export { login as authenticate } from \"@demo/sdk/auth\""
        }));
        assert!(!workspace_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:export * from \"@/session\""));
    }

    #[test]
    fn ts_export_following_workspace_only_prefers_conditioned_sources_and_wildcard_exports() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(src.join("features")).expect("failed to create src/features");
        fs::create_dir_all(workspace.path().join("dist/features"))
            .expect("failed to create dist/features");

        fs::write(
            workspace.path().join("package.json"),
            r#"
            {
              "name": "@demo/sdk",
              "exports": {
                ".": {
                  "default": "./dist/index.js",
                  "import": "./src/root.ts"
                },
                "./features/*": {
                  "default": "./dist/features/*.js",
                  "import": "./src/features/*.ts"
                }
              }
            }
        "#,
        )
        .expect("failed to write package.json");

        fs::write(
            src.join("root.ts"),
            r#"
            export function createKit(name: string): string {
              return name;
            }
        "#,
        )
        .expect("failed to write root fixture");

        fs::write(
            src.join("features/session.ts"),
            r#"
            export interface SessionFeature {
              token: string;
            }
        "#,
        )
        .expect("failed to write feature fixture");

        fs::write(
            src.join("index.ts"),
            r#"
            export { createKit } from "@demo/sdk";
            export * from "@demo/sdk/features/session";
        "#,
        )
        .expect("failed to write index fixture");

        let report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/index.ts".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::WorkspaceOnly,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:function createKit(name: string): string"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:interface SessionFeature"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("dist/index.js")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("dist/features")));
    }

    #[test]
    fn ts_export_following_workspace_only_resolves_package_imports_maps() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(src.join("internal/features")).expect("failed to create src/internal");
        fs::create_dir_all(workspace.path().join("dist/internal/features"))
            .expect("failed to create dist/internal/features");

        fs::write(
            workspace.path().join("package.json"),
            r##"
            {
              "name": "@demo/sdk",
              "imports": {
                "#core": {
                  "default": "./dist/internal/core.js",
                  "import": "./src/internal/core.ts"
                },
                "#features/*": {
                  "default": "./dist/internal/features/*.js",
                  "import": "./src/internal/features/*.ts"
                }
              }
            }
        "##,
        )
        .expect("failed to write package.json");

        fs::write(
            src.join("internal/core.ts"),
            r#"
            export function boot(target: string): string {
              return target;
            }
        "#,
        )
        .expect("failed to write core fixture");

        fs::write(
            src.join("internal/features/session.ts"),
            r#"
            export interface InternalSession {
              token: string;
            }
        "#,
        )
        .expect("failed to write feature fixture");

        fs::write(
            src.join("index.ts"),
            r##"
            export { boot } from "#core";
            export * from "#features/session";
        "##,
        )
        .expect("failed to write index fixture");

        let report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/index.ts".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::WorkspaceOnly,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:function boot(target: string): string"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:interface InternalSession"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:export { boot } from \"#core\""));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ts/js:export * from \"#features/session\""));
    }

    #[test]
    fn ts_export_following_workspace_only_resolves_monorepo_package_self_references() {
        let workspace = TempWorkspace::new();
        let package_src = workspace.path().join("packages/sdk/src");
        fs::create_dir_all(&package_src).expect("failed to create packages/sdk/src");

        fs::write(
            workspace.path().join("package.json"),
            r#"
            {
              "private": true,
              "workspaces": ["packages/*"]
            }
        "#,
        )
        .expect("failed to write monorepo package.json");

        fs::write(
            workspace.path().join("packages/sdk/package.json"),
            r#"
            {
              "name": "@demo/sdk",
              "exports": {
                "./auth": "./src/auth.ts"
              }
            }
        "#,
        )
        .expect("failed to write package fixture");

        fs::write(
            package_src.join("auth.ts"),
            r#"
            export function login(username: string): string {
              return username;
            }
        "#,
        )
        .expect("failed to write auth fixture");

        fs::write(
            package_src.join("index.ts"),
            r#"
            export { login as authenticate } from "@demo/sdk/auth";
        "#,
        )
        .expect("failed to write index fixture");

        let default_report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["packages/sdk/src/index.ts".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::RelativeOnly,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(default_report.public_api.iter().any(|entry| {
            entry.signature == "ts/js:export { login as authenticate } from \"@demo/sdk/auth\""
        }));

        let workspace_report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["packages/sdk/src/index.ts".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::WorkspaceOnly,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(workspace_report.public_api.iter().any(
            |entry| entry.signature == "ts/js:function authenticate(username: string): string"
        ));
        assert!(!workspace_report.public_api.iter().any(|entry| {
            entry.signature == "ts/js:export { login as authenticate } from \"@demo/sdk/auth\""
        }));
    }

    #[test]
    fn go_export_following_package_only_expands_file_roots_to_same_package_siblings() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.go"),
            r#"
            package demo

            func Connect(target string) error {
                return nil
            }
        "#,
        )
        .expect("failed to write go api fixture");

        fs::write(
            src.join("types.go"),
            r#"
            package demo

            type Session struct {
                Token string
            }
        "#,
        )
        .expect("failed to write go types fixture");

        fs::write(
            src.join("api_test.go"),
            r#"
            package demo

            const TestHelper string = "ignored"
        "#,
        )
        .expect("failed to write go test fixture");

        let default_report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/api.go".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(default_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:func Connect(target string) error"));
        assert!(!default_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:type Session struct"));

        let package_report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/api.go".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::PackageOnly,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(package_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:func Connect(target string) error"));
        assert!(package_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:type Session struct"));
        assert!(package_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:field Session.Token string"));
        assert!(!package_report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("TestHelper")));
    }

    #[test]
    fn tree_sitter_captures_default_export_without_body_noise() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.ts"),
            r#"
            export default function login(
              username: string,
              password: string,
            ): Promise<string> {
              return Promise.resolve(`${username}:${password}`);
            }
        "#,
        )
        .expect("failed to write ts fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.public_api.iter().any(|entry| {
            entry.signature
                == "ts/js:export default function login(username: string, password: string): Promise<string>"
        }));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("Promise.resolve")));
    }

    #[test]
    fn tree_sitter_captures_go_exported_functions_and_methods() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.go"),
            r#"
            package demo

            type service struct{}
            type Transport struct{}
            type Reader interface {
                Read(p []byte) (int, error)
            }
            type Session struct {
                *Transport
                Token string
                hidden string
            }

            type Contract interface {
                Reader
                Sync(token string) error
                hidden() error
            }

            type SessionID = string

            const Version string = "v1"
            const internalVersion = "v0"
            var DefaultTimeout int = 30
            var internalClient = service{}

            func ExportedLogin(username string) string {
                return username
            }

            func unexported() {}

            func (s *service) hidden(target string) error {
                return nil
            }

            func (s *service) Connect(target string) error {
                return nil
            }
        "#,
        )
        .expect("failed to write go fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:func ExportedLogin(username string) string"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:func(s *service) Connect(target string) error"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:type Session struct"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:embed Session *Transport"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:field Session.Token string"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:type Contract interface"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:interface-type Contract Reader"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:interface Contract.Sync(token string) error"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:type SessionID = string"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:const Version string"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "go:var DefaultTimeout int"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("unexported")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("hidden")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("internalVersion")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("internalClient")));
    }

    #[test]
    fn tree_sitter_captures_python_public_defs_and_respects_all() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.py"),
            r#"
            API_VERSION: str = "v1"
            __all__ = ("login", "Worker")
            type SessionToken = str
            type _InternalToken = str
            _INTERNAL = "hidden"

            class Worker:
                STATUS: str = "ready"
                _HIDDEN_STATUS = "hidden"

                @staticmethod
                def run_job(name: str) -> str:
                    def helper() -> str:
                        return name
                    return helper()

                def _hidden(self) -> str:
                    return "hidden"

            class _InternalWorker:
                STATUS: str = "hidden"

                def run_job(name: str) -> str:
                    return name

            @decorator
            def login(username: str) -> str:
                return username

            def _internal() -> str:
                return "hidden"
        "#,
        )
        .expect("failed to write python fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:const __all__"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:class Worker"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:const Worker.STATUS: str"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:def Worker.run_job(name: str) -> str"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:def login(username: str) -> str"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("API_VERSION")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("SessionToken")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("helper")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("@decorator")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("_internal")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("_InternalWorker")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("_InternalToken")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("_INTERNAL")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("_HIDDEN_STATUS")));
    }

    #[test]
    fn tree_sitter_captures_python_all_composition_via_aliases_and_augmented_assignments() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.py"),
            r#"
            API_VERSION: str = "v1"
            CORE_EXPORTS = ("login",)
            EXTRA_EXPORTS = ["Worker"]
            TYPE_EXPORT = "SessionToken"
            ALL_EXPORTS = CORE_EXPORTS + EXTRA_EXPORTS
            ALL_EXPORTS += (TYPE_EXPORT,)
            __all__ = ALL_EXPORTS

            type SessionToken = str
            type _InternalToken = str

            class Worker:
                STATUS: str = "ready"

                def run_job(name: str) -> str:
                    return name

            def login(username: str) -> str:
                return username

            def _internal() -> str:
                return "hidden"
        "#,
        )
        .expect("failed to write python fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:const __all__"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:class Worker"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:const Worker.STATUS: str"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:def Worker.run_job(name: str) -> str"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:def login(username: str) -> str"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:type SessionToken = str"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("API_VERSION")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("CORE_EXPORTS")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("EXTRA_EXPORTS")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("TYPE_EXPORT")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("_internal")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("_InternalToken")));
    }

    #[test]
    fn tree_sitter_captures_python_explicit_import_reexports_and_splat_exports() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.py"),
            r#"
            CORE_EXPORTS = ("authorize",)
            __all__ = [*CORE_EXPORTS, "Worker", "tokens"]

            from auth.core import login as authorize
            from auth.models import Worker
            import auth.tokens as tokens
            import typing as _typing

            def _internal() -> str:
                return "hidden"
        "#,
        )
        .expect("failed to write python fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:const __all__"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:from auth.core import login as authorize"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:from auth.models import Worker"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:import auth.tokens as tokens"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("CORE_EXPORTS")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("_typing")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("_internal")));
    }

    #[test]
    fn tree_sitter_captures_python_cross_module_all_aliases_and_wildcard_reexports() {
        let workspace = TempWorkspace::new();
        let auth = workspace.path().join("auth");
        fs::create_dir_all(&auth).expect("failed to create auth package");

        fs::write(
            auth.join("core.py"),
            r#"
            __all__ = ("login", "SessionToken")

            type SessionToken = str

            def login(username: str) -> str:
                return username

            def _hidden() -> str:
                return "hidden"
        "#,
        )
        .expect("failed to write auth core fixture");

        fs::write(
            workspace.path().join("api.py"),
            r#"
            from auth.core import __all__ as CORE_EXPORTS
            from auth.core import *

            __all__ = [*CORE_EXPORTS, "Worker"]

            class Worker:
                STATUS: str = "ready"
        "#,
        )
        .expect("failed to write python facade fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:const __all__"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:class Worker"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:const Worker.STATUS: str"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:from auth.core import login"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:from auth.core import SessionToken"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("CORE_EXPORTS")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("_hidden")));
    }

    #[test]
    fn python_module_roots_enable_nonstandard_workspace_package_resolution() {
        let workspace = TempWorkspace::new();
        let pkg = workspace.path().join("app/pkg");
        fs::create_dir_all(&pkg).expect("failed to create python package");

        fs::write(
            pkg.join("core.py"),
            r#"
            __all__ = ("login",)

            def login(username: str) -> str:
                return username
        "#,
        )
        .expect("failed to write package core fixture");

        fs::write(
            workspace.path().join("app/api.py"),
            r#"
            from pkg.core import __all__ as CORE_EXPORTS
            from pkg.core import *

            __all__ = CORE_EXPORTS
        "#,
        )
        .expect("failed to write python facade fixture");

        let default_report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");
        assert!(!default_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:from pkg.core import login"));

        let rooted_report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: Vec::new(),
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: vec!["app".to_string()],
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(rooted_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:from pkg.core import login"));
    }

    #[test]
    fn python_export_following_off_disables_cross_module_workspace_resolution() {
        let workspace = TempWorkspace::new();
        let pkg = workspace.path().join("app/pkg");
        fs::create_dir_all(&pkg).expect("failed to create python package");

        fs::write(
            pkg.join("core.py"),
            r#"
            __all__ = ("login",)

            def login(username: str) -> str:
                return username
        "#,
        )
        .expect("failed to write package core fixture");

        fs::write(
            workspace.path().join("app/api.py"),
            r#"
            from pkg.core import __all__ as CORE_EXPORTS
            from pkg.core import *

            __all__ = CORE_EXPORTS
        "#,
        )
        .expect("failed to write python facade fixture");

        let report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: Vec::new(),
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Off,
                python_module_roots: vec!["app".to_string()],
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:from pkg.core import login"));
    }

    #[test]
    fn tree_sitter_python_falls_back_when_all_is_not_parseable() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.py"),
            r#"
            API_VERSION: str = "v1"
            __all__ = _build_exports()
            type SessionToken = str

            def _build_exports():
                return ("login", "Worker")

            class Worker:
                STATUS: str = "ready"

                def run_job(name: str) -> str:
                    return name

            def login(username: str) -> str:
                return username
        "#,
        )
        .expect("failed to write python fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:const API_VERSION: str"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:const __all__"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:type SessionToken = str"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:class Worker"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:const Worker.STATUS: str"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:def Worker.run_job(name: str) -> str"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "python:def login(username: str) -> str"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("_build_exports")));
    }

    #[test]
    fn tree_sitter_captures_java_public_types_methods_fields_and_constants() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("AuthApi.java"),
            r#"
            package demo;

            @Deprecated
            public record Session(String token) {}

            public class AuthApi {
                public static final String VERSION = "v1";
                public String status = "ready";

                @Deprecated
                public String login(String username) {
                    return username;
                }

                public interface Contract {
                    String sync(String username);
                    String STATE = "ready";
                }

                void hidden() {}

                public static class Nested {}
            }
        "#,
        )
        .expect("failed to write java fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "java:type public record demo.Session(String token)"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "java:type public class demo.AuthApi"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "java:type public static class demo.AuthApi.Nested"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "java:type public interface demo.AuthApi.Contract"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature
                == "java:field public static final String demo.AuthApi.VERSION"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "java:field public String demo.AuthApi.status"));
        assert!(report.public_api.iter().any(|entry| entry.signature
            == "java:method public String demo.AuthApi.login(String username)"));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature
                == "java:method public String demo.AuthApi.Contract.sync(String username)"
        }));
        assert!(report.public_api.iter().any(|entry| entry.signature
            == "java:const public static final String demo.AuthApi.Contract.STATE"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("@Deprecated")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("hidden")));
    }

    #[test]
    fn dart_regex_captures_public_types_functions_and_fields() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.dart"),
            r#"
            library demo;

            class AuthApi {
              static const String VERSION = 'v1';
              String status = 'ready';

              String login(String username) {
                return username;
              }

              void _hidden() {}
            }

            String greet(String name) => 'Hi';
        "#,
        )
        .expect("failed to write dart fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.public_api.iter().any(|entry| {
            entry.signature.starts_with("dart:type ")
                && entry.signature.contains("demo.")
                && entry.signature.contains("class AuthApi")
        }));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature.starts_with("dart:field ")
                && entry.signature.contains("demo.")
                && entry.signature.contains("VERSION")
        }));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature.starts_with("dart:field ")
                && entry.signature.contains("demo.")
                && entry.signature.contains("status")
        }));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature.starts_with("dart:function ")
                && entry.signature.contains("demo.")
                && entry.signature.contains("login(String username)")
        }));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature.starts_with("dart:function ")
                && entry.signature.contains("demo.")
                && entry.signature.contains("greet(String name)")
        }));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("_hidden")));
    }

    #[test]
    fn tree_sitter_captures_kotlin_public_declarations_properties_and_constants() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.kt"),
            r#"
            package demo.auth

            const val API_VERSION: String = "v1"

            public class AuthApi {
                val token: String = "ready"
                var status: String = "active"

                fun login(username: String): String {
                    return username
                }

                interface Contract {
                    val sessionToken: String
                }

                private fun hidden(): String {
                    return "hidden"
                }

                private val hiddenToken: String = "hidden"

                data class Session(val token: String)
            }

            internal object InternalDefaults

            suspend fun load(token: String): String {
                return token
            }

            object Defaults {
                val timeout: Int = 5
            }
        "#,
        )
        .expect("failed to write kotlin fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "kotlin:public class demo.auth.AuthApi"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "kotlin:const val demo.auth.API_VERSION: String"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "kotlin:val demo.auth.AuthApi.token: String"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "kotlin:var demo.auth.AuthApi.status: String"));
        assert!(report.public_api.iter().any(|entry| entry.signature
            == "kotlin:fun demo.auth.AuthApi.login(username: String): String"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "kotlin:interface demo.auth.AuthApi.Contract"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature
                == "kotlin:val demo.auth.AuthApi.Contract.sessionToken: String"));
        assert!(report.public_api.iter().any(|entry| entry.signature
            == "kotlin:data class demo.auth.AuthApi.Session(val token: String)"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature
                == "kotlin:suspend fun demo.auth.load(token: String): String"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "kotlin:object demo.auth.Defaults"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "kotlin:val demo.auth.Defaults.timeout: Int"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("hidden")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("InternalDefaults")));
    }

    #[test]
    fn tree_sitter_captures_csharp_public_types_methods_fields_and_properties_without_attributes() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("Api.cs"),
            r#"
            namespace Demo;

            [Obsolete]
            public record Session(string Token);

            public class AuthApi {
                public static readonly string Version = "v1";
                public const string STATUS_READY = "ready";
                public string DisplayName { get; private set; }

                [Obsolete]
                public static string Login(string username) {
                    return username;
                }

                public interface Contract {
                    string Sync(string username);
                    string State { get; }
                }

                private static string Hidden(string username) {
                    return username;
                }

                private string HiddenName { get; set; }

                public struct Result { }
            }
        "#,
        )
        .expect("failed to write csharp fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(
            report
                .public_api
                .iter()
                .any(|entry| entry.signature
                    == "csharp:type public record Demo.Session(string Token)")
        );
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "csharp:type public class Demo.AuthApi"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "csharp:type public interface Demo.AuthApi.Contract"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "csharp:type public struct Demo.AuthApi.Result"));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "csharp:field public static readonly string Demo.AuthApi.Version"
        }));
        assert!(
            report
                .public_api
                .iter()
                .any(|entry| entry.signature
                    == "csharp:const public string Demo.AuthApi.STATUS_READY")
        );
        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "csharp:property public string Demo.AuthApi.DisplayName { get }"
        }));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature
                == "csharp:method public static string Demo.AuthApi.Login(string username)"
        }));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature
                == "csharp:method public string Demo.AuthApi.Contract.Sync(string username)"
        }));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "csharp:property public string Demo.AuthApi.Contract.State { get }"
        }));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("[Obsolete]")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("Hidden")));
    }

    #[test]
    fn tree_sitter_captures_swift_public_types_properties_and_protocol_requirements() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("Api.swift"),
            r#"
            let example = """
            // @mvs-feature("fake_feature")
            """

            // @mvs-feature("swift_bridge")
            // @mvs-protocol("swift-api-v1")
            public struct Session {}

            public protocol SessionContract {
                var token: String { get }
                func renew(target: String) -> Bool
            }

            public class AuthApi {
                public var status: String {
                    "ready"
                }

                public func login(username: String) -> String {
                    username
                }

                private var hiddenStatus: String {
                    "hidden"
                }

                private func hidden() -> String {
                    "hidden"
                }
            }
        "#,
        )
        .expect("failed to write swift fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.contains("swift_bridge"));
        assert!(report.protocol_tags.contains("swift-api-v1"));
        assert!(!report.feature_tags.contains("fake_feature"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "swift:public struct Session"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "swift:public protocol SessionContract"));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "swift:public var SessionContract.token: String { get }"
        }));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "swift:public func SessionContract.renew(target: String) -> Bool"
        }));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "swift:public class AuthApi"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "swift:public var AuthApi.status: String"));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "swift:public func AuthApi.login(username: String) -> String"
        }));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("hidden")));
    }

    #[test]
    fn tree_sitter_captures_php_public_api_constants_properties_and_hash_comments() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("Api.php"),
            r#"
            <?php

            # @mvs-feature("php_jobs")
            /* @mvs-protocol("php-worker-v1") */
            #[Deprecated]
            function login(string $username): string {
                return $username;
            }

            const VERSION = "1";

            #[Deprecated]
            final class AuthApi {
                public const STATUS_READY = "ready";
                public readonly string $token;
                public static string $sharedName;
                private string $secret;

                #[Deprecated]
                public static function run(string $name): string {
                    return $name;
                }

                private function hidden(string $name): string {
                    return $name;
                }
            }

            interface Contract {
                const SYNC = "sync";
                public function sync(string $token): void;
            }
        "#,
        )
        .expect("failed to write php fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.contains("php_jobs"));
        assert!(report.protocol_tags.contains("php-worker-v1"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:function login(string $username): string"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:const VERSION"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:final class AuthApi"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:public const AuthApi::STATUS_READY"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:public readonly string AuthApi.$token"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:public static string AuthApi.$sharedName"));
        assert!(report.public_api.iter().any(|entry| {
            entry.signature == "php:public static function AuthApi.run(string $name): string"
        }));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:interface Contract"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:public const Contract::SYNC"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "php:function Contract.sync(string $token): void"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("#[Deprecated]")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("hidden")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("$secret")));
    }

    #[test]
    fn tree_sitter_captures_ruby_public_api_without_heredoc_or_private_visibility_noise() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.rb"),
            r##"
            fixture = <<~DOC
            # @mvs-feature("fake_feature")
            # @mvs-protocol("fake_protocol")
            DOC

            # @mvs-feature("ruby_bridge")
            # @mvs-protocol("ruby-api-v1")
            module Demo
              VERSION = "v1"
              SECRET = "hidden"
              private_constant :SECRET
              public_constant :SECRET

              module_function

              def build(token)
                token
              end

              extend self

              def ping(target)
                target
              end

              def status(label)
                label
              end

              module_function :ping

              class AuthApi < BaseApi
                TIMEOUT = 30
                PRIVATE_TOKEN = "hidden"
                private_constant :PRIVATE_TOKEN
                public_constant :PRIVATE_TOKEN

                attr_reader :token, :status
                attr_accessor :mode

                def login(username)
                  username
                end

                def self.publish(target)
                  target
                end

                class << self
                  def connect(target)
                    target
                  end
                end

                private_class_method :connect

                private

                def hidden(secret)
                  secret
                end
              end
            end
        "##,
        )
        .expect("failed to write ruby fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.contains("ruby_bridge"));
        assert!(report.protocol_tags.contains("ruby-api-v1"));
        assert!(!report.feature_tags.contains("fake_feature"));
        assert!(!report.protocol_tags.contains("fake_protocol"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:module Demo"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:const Demo::VERSION"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:const Demo::SECRET"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:class AuthApi < BaseApi"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:const Demo::AuthApi::TIMEOUT"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:const Demo::AuthApi::PRIVATE_TOKEN"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:def Demo.build(token)"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:def Demo.ping(target)"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:def Demo#status(label)"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:def Demo.status(label)"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:attr_reader Demo::AuthApi#token"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:attr_reader Demo::AuthApi#status"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:attr_accessor Demo::AuthApi#mode"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:def Demo::AuthApi#login(username)"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:def Demo::AuthApi.publish(target)"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("hidden")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:def Demo#ping(target)"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:def Demo::AuthApi.connect(target)"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("Demo::AuthApi#hidden")));
    }

    #[test]
    fn ruby_export_following_off_keeps_file_local_visibility_without_macro_promotion() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.rb"),
            r#"
            module Demo
              SECRET = "hidden"
              private_constant :SECRET

              module_function

              def build(token)
                token
              end

              extend self

              def ping(target)
                target
              end
            end
        "#,
        )
        .expect("failed to write ruby fixture");

        let heuristic_report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");
        assert!(!heuristic_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:const Demo::SECRET"));
        assert!(heuristic_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:def Demo.build(token)"));
        assert!(heuristic_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:def Demo.ping(target)"));

        let off_report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: Vec::new(),
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Off,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(off_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:const Demo::SECRET"));
        assert!(off_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:def Demo#build(token)"));
        assert!(off_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:def Demo#ping(target)"));
        assert!(!off_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:def Demo.build(token)"));
        assert!(!off_report
            .public_api
            .iter()
            .any(|entry| entry.signature == "ruby:def Demo.ping(target)"));
    }

    #[test]
    fn tree_sitter_captures_lua_global_functions_and_module_exports() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("Api.lua"),
            r#"
            local fixture = [[
            -- @mvs-feature("fake_feature")
            -- @mvs-protocol("fake_protocol")
            ]]

            -- @mvs-feature("lua_bridge")
            -- @mvs-protocol("lua-module-v1")
            function connect(target)
                return target ~= ""
            end

            local Api = {
                ping = function(target)
                    return target ~= ""
                end,
                version = "v1",
            }

            Api.connect = function(target)
                return target ~= ""
            end

            function Api:refresh(token)
                return token ~= ""
            end

            local Internal = {}

            function Internal.hidden()
                return false
            end

            local function hidden()
                return false
            end

            return Api
        "#,
        )
        .expect("failed to write lua fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.contains("lua_bridge"));
        assert!(report.protocol_tags.contains("lua-module-v1"));
        assert!(!report.feature_tags.contains("fake_feature"));
        assert!(!report.protocol_tags.contains("fake_protocol"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "lua:function Api.ping(target)"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "lua:field Api.version"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "lua:function Api.connect(target)"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "lua:function Api:refresh(token)"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("hidden")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("Internal.hidden")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature == "lua:function connect(target)"));
    }

    #[test]
    fn lua_export_following_off_disables_returned_root_module_following() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("Api.lua"),
            r#"
            function connect(target)
                return target ~= ""
            end

            local Api = {
                ping = function(target)
                    return target ~= ""
                end,
            }

            Api.connect = function(target)
                return target ~= ""
            end

            return Api
        "#,
        )
        .expect("failed to write lua fixture");

        let report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: Vec::new(),
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Off,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "lua:function connect(target)"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature == "lua:function Api.ping(target)"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature == "lua:function Api.connect(target)"));
    }

    #[test]
    fn tree_sitter_captures_lua_returned_function_identifier_as_export() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("Api.lua"),
            r#"
            function connect(target)
                return target ~= ""
            end

            function hidden(target)
                return target == ""
            end

            return connect
        "#,
        )
        .expect("failed to write lua fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "lua:function connect(target)"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("hidden")));
    }

    #[test]
    fn tree_sitter_captures_luau_global_functions_module_exports_and_export_types() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("Api.luau"),
            r#"
            local fixture = [[
            -- @mvs-feature("fake_feature")
            ]]

            -- @mvs-feature("luau_bridge")
            --[[ @mvs-protocol("luau-module-v1") ]]
            export type Session = {
                token: string,
            }

            function connect(target: string): boolean
                return target ~= ""
            end

            local Api = {
                ping = function(target: string): boolean
                    return target ~= ""
                end,
                version = "v1",
            }

            Api.connect = function(target: string): boolean
                return target ~= ""
            end

            function Api:refresh(token: string): boolean
                return token ~= ""
            end

            local Internal = {}

            function Internal.hidden(): boolean
                return false
            end

            local function hidden(): boolean
                return false
            end

            return Api
        "#,
        )
        .expect("failed to write luau fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report.feature_tags.contains("luau_bridge"));
        assert!(report.protocol_tags.contains("luau-module-v1"));
        assert!(!report.feature_tags.contains("fake_feature"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "luau:export type Session={ token: string }"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "luau:function Api.ping(target: string): boolean"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "luau:field Api.version"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "luau:function Api.connect(target: string): boolean"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "luau:function Api:refresh(token: string): boolean"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("hidden")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("Internal.hidden")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature == "luau:function connect(target: string): boolean"));
    }

    #[test]
    fn lua_export_following_returned_root_only_requires_runtime_return_boundary() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("Api.luau"),
            r#"
            export type Session = {
                token: string,
            }

            function connect(target: string): boolean
                return target ~= ""
            end
        "#,
        )
        .expect("failed to write luau fixture");

        let report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: Vec::new(),
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::ReturnedRootOnly,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature == "luau:export type Session={ token: string }"));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature == "luau:function connect(target: string): boolean"));
    }

    #[test]
    fn parser_backed_adapters_share_comment_aware_contract_smoke_coverage() {
        let cases = [
            ParserAdapterCase {
                file_name: "api.ts",
                source: r#"
                const fixture = `
                // @mvs-feature("fake_feature")
                // @mvs-protocol("fake_protocol")
                export function fakeLogin(username: string): Promise<string> {
                  return Promise.resolve(username)
                }
                `;

                // @mvs-feature("ts_bridge")
                // @mvs-protocol("ts-api-v1")
                export function login(username: string): Promise<string> {
                  return Promise.resolve(username)
                }
            "#,
                expected_feature: "ts_bridge",
                expected_protocol: "ts-api-v1",
                expected_public_api: &["ts/js:function login(username: string): Promise<string>"],
                rejected_public_api_fragments: &["fakeLogin"],
            },
            ParserAdapterCase {
                file_name: "api.go",
                source: r#"
                package demo

                var fixture = `
                // @mvs-feature("fake_feature")
                // @mvs-protocol("fake_protocol")
                `

                // @mvs-feature("go_bridge")
                // @mvs-protocol("go-api-v1")
                type Transport struct{}

                type Session struct {
                    *Transport
                    Token string
                    hidden string
                }

                const Version string = "v1"
                var DefaultTimeout int = 30

                func Connect(target string) error {
                    return nil
                }

                func hidden(target string) error {
                    return nil
                }
            "#,
                expected_feature: "go_bridge",
                expected_protocol: "go-api-v1",
                expected_public_api: &[
                    "go:type Session struct",
                    "go:embed Session *Transport",
                    "go:field Session.Token string",
                    "go:const Version string",
                    "go:var DefaultTimeout int",
                    "go:func Connect(target string) error",
                ],
                rejected_public_api_fragments: &["hidden"],
            },
            ParserAdapterCase {
                file_name: "api.py",
                source: r#"
                fixture = """
                # @mvs-feature("fake_feature")
                # @mvs-protocol("fake_protocol")
                """

                # @mvs-feature("python_bridge")
                # @mvs-protocol("python-api-v1")
                API_VERSION: str = "v1"
                CORE_EXPORTS = ("authorize",)
                __all__ = [*CORE_EXPORTS, "Worker"]
                type SessionToken = str

                from auth.core import login as authorize

                class Worker:
                    STATUS: str = "ready"
                    pass

                def _hidden() -> str:
                    return "hidden"
            "#,
                expected_feature: "python_bridge",
                expected_protocol: "python-api-v1",
                expected_public_api: &[
                    "python:const __all__",
                    "python:class Worker",
                    "python:const Worker.STATUS: str",
                    "python:from auth.core import login as authorize",
                ],
                rejected_public_api_fragments: &[
                    "_hidden",
                    "API_VERSION",
                    "SessionToken",
                    "CORE_EXPORTS",
                    "typing",
                ],
            },
            ParserAdapterCase {
                file_name: "AuthApi.java",
                source: r#"
                package demo;

                class Fixture {
                    String example = """
                    // @mvs-feature("fake_feature")
                    // @mvs-protocol("fake_protocol")
                    """;
                }

                // @mvs-feature("java_bridge")
                // @mvs-protocol("java-api-v1")
                public class AuthApi {
                    public static final String VERSION = "v1";

                    public String login(String username) {
                        return username;
                    }

                    public interface Contract {
                        String sync(String username);
                    }

                    private String hidden(String username) {
                        return username;
                    }
                }
            "#,
                expected_feature: "java_bridge",
                expected_protocol: "java-api-v1",
                expected_public_api: &[
                    "java:type public class demo.AuthApi",
                    "java:field public static final String demo.AuthApi.VERSION",
                    "java:method public String demo.AuthApi.login(String username)",
                    "java:type public interface demo.AuthApi.Contract",
                    "java:method public String demo.AuthApi.Contract.sync(String username)",
                ],
                rejected_public_api_fragments: &["hidden"],
            },
            ParserAdapterCase {
                file_name: "api.kt",
                source: r#"
                package demo.auth

                val fixture = """
                // @mvs-feature("fake_feature")
                // @mvs-protocol("fake_protocol")
                """

                // @mvs-feature("kotlin_bridge")
                // @mvs-protocol("kotlin-api-v1")
                const val API_VERSION: String = "v1"

                class AuthApi {
                    val token: String = "ready"

                    fun login(username: String): String {
                        return username
                    }

                    private fun hidden(): String {
                        return "hidden"
                    }
                }
            "#,
                expected_feature: "kotlin_bridge",
                expected_protocol: "kotlin-api-v1",
                expected_public_api: &[
                    "kotlin:const val demo.auth.API_VERSION: String",
                    "kotlin:class demo.auth.AuthApi",
                    "kotlin:val demo.auth.AuthApi.token: String",
                    "kotlin:fun demo.auth.AuthApi.login(username: String): String",
                ],
                rejected_public_api_fragments: &["hidden"],
            },
            ParserAdapterCase {
                file_name: "Api.cs",
                source: r#"
                namespace Demo;

                var fixture = @"
                // @mvs-feature(""fake_feature"")
                // @mvs-protocol(""fake_protocol"")
                ";

                // @mvs-feature("csharp_bridge")
                // @mvs-protocol("csharp-api-v1")
                public class AuthApi {
                    public static readonly string Version = "v1";
                    public string DisplayName { get; private set; }

                    public static string Login(string username) {
                        return username;
                    }

                    private static string Hidden(string username) {
                        return username;
                    }
                }
            "#,
                expected_feature: "csharp_bridge",
                expected_protocol: "csharp-api-v1",
                expected_public_api: &[
                    "csharp:type public class Demo.AuthApi",
                    "csharp:field public static readonly string Demo.AuthApi.Version",
                    "csharp:property public string Demo.AuthApi.DisplayName { get }",
                    "csharp:method public static string Demo.AuthApi.Login(string username)",
                ],
                rejected_public_api_fragments: &["Hidden"],
            },
            ParserAdapterCase {
                file_name: "Api.php",
                source: r#"
                <?php

                $fixture = "
                # @mvs-feature(\"fake_feature\")
                # @mvs-protocol(\"fake_protocol\")
                ";

                # @mvs-feature("php_bridge")
                # @mvs-protocol("php-api-v1")
                function login(string $username): string {
                    return $username;
                }

                final class InternalApi {
                    private function hidden(string $username): string {
                        return $username;
                    }
                }
            "#,
                expected_feature: "php_bridge",
                expected_protocol: "php-api-v1",
                expected_public_api: &["php:function login(string $username): string"],
                rejected_public_api_fragments: &["hidden"],
            },
            ParserAdapterCase {
                file_name: "api.rb",
                source: r##"
                fixture = <<~DOC
                # @mvs-feature("fake_feature")
                # @mvs-protocol("fake_protocol")
                DOC

                # @mvs-feature("ruby_bridge")
                # @mvs-protocol("ruby-api-v1")
                module Demo
                  VERSION = "v1"
                  SECRET = "hidden"
                  private_constant :SECRET
                  public_constant :SECRET

                  extend self

                  class AuthApi
                    attr_reader :token

                    def login(username)
                      username
                    end

                    def self.publish(target)
                      target
                    end

                    private_class_method :publish

                    private

                    def hidden(secret)
                      secret
                    end
                  end

                  def ping(target)
                    target
                  end

                  module_function :ping
                end
            "##,
                expected_feature: "ruby_bridge",
                expected_protocol: "ruby-api-v1",
                expected_public_api: &[
                    "ruby:module Demo",
                    "ruby:const Demo::VERSION",
                    "ruby:const Demo::SECRET",
                    "ruby:class AuthApi",
                    "ruby:attr_reader Demo::AuthApi#token",
                    "ruby:def Demo::AuthApi#login(username)",
                    "ruby:def Demo.ping(target)",
                ],
                rejected_public_api_fragments: &["hidden", "AuthApi.publish", "Demo#ping"],
            },
            ParserAdapterCase {
                file_name: "Api.swift",
                source: r#"
                let fixture = """
                // @mvs-feature("fake_feature")
                // @mvs-protocol("fake_protocol")
                """

                // @mvs-feature("swift_bridge")
                // @mvs-protocol("swift-api-v1")
                public struct Session {
                    public let token: String
                }
            "#,
                expected_feature: "swift_bridge",
                expected_protocol: "swift-api-v1",
                expected_public_api: &[
                    "swift:public struct Session",
                    "swift:public let Session.token: String",
                ],
                rejected_public_api_fragments: &["fake_feature"],
            },
            ParserAdapterCase {
                file_name: "Api.lua",
                source: r#"
                local fixture = [[
                -- @mvs-feature("fake_feature")
                -- @mvs-protocol("fake_protocol")
                ]]

                -- @mvs-feature("lua_bridge")
                -- @mvs-protocol("lua-api-v1")
                function connect(target)
                    return target ~= ""
                end

                local function hidden()
                    return false
                end
            "#,
                expected_feature: "lua_bridge",
                expected_protocol: "lua-api-v1",
                expected_public_api: &["lua:function connect(target)"],
                rejected_public_api_fragments: &["hidden"],
            },
            ParserAdapterCase {
                file_name: "Api.luau",
                source: r#"
                local fixture = [[
                -- @mvs-feature("fake_feature")
                -- @mvs-protocol("fake_protocol")
                ]]

                -- @mvs-feature("luau_bridge")
                -- @mvs-protocol("luau-api-v1")
                function connect(target: string): boolean
                    return target ~= ""
                end

                local function hidden(): boolean
                    return false
                end
            "#,
                expected_feature: "luau_bridge",
                expected_protocol: "luau-api-v1",
                expected_public_api: &["luau:function connect(target: string): boolean"],
                rejected_public_api_fragments: &["hidden"],
            },
        ];

        for case in cases {
            assert_parser_adapter_case(case);
        }
    }

    #[test]
    fn public_api_roots_scope_api_inventory_without_hiding_tags() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.ts"),
            r#"
            // @mvs-feature("auth")
            // @mvs-protocol("auth-api")
            export function login(username: string): Promise<string> {
              return Promise.resolve(username)
            }
        "#,
        )
        .expect("failed to write api fixture");

        fs::write(
            src.join("internal.ts"),
            r#"
            // @mvs-feature("background_jobs")
            // @mvs-protocol("jobs-api")
            export function runJob(name: string): string {
              return name
            }
        "#,
        )
        .expect("failed to write internal fixture");

        let report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/api.ts".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(report.feature_tags.contains("auth"));
        assert!(report.feature_tags.contains("background_jobs"));
        assert!(report.protocol_tags.contains("auth-api"));
        assert!(report.protocol_tags.contains("jobs-api"));
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("login")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("runJob")));
    }

    #[test]
    fn exclude_paths_skip_tag_and_api_scans() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(src.join("generated")).expect("failed to create generated dir");

        fs::write(
            src.join("generated/client.ts"),
            r#"
            // @mvs-feature("generated_api")
            // @mvs-protocol("generated-api")
            export function request(path: string): string {
              return path
            }
        "#,
        )
        .expect("failed to write generated fixture");

        let report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: vec!["src/generated".to_string()],
                public_api_roots: Vec::new(),
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: Vec::new(),
                public_api_excludes: Vec::new(),
            },
        )
        .expect("crawler failed");

        assert!(report.feature_tags.is_empty());
        assert!(report.protocol_tags.is_empty());
        assert!(report.public_api.is_empty());
        assert!(report.excluded_paths.iter().any(|decision| {
            decision.path == "src/generated"
                && decision.kind == ExcludedPathKind::Directory
                && decision.reason == "scan_policy_exclude_path"
                && decision.rule.as_deref() == Some("src/generated")
        }));
    }

    #[test]
    fn ignores_exports_inside_template_strings() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.ts"),
            r#"
            const fixture = `
            export function fakeLogin(username: string): Promise<string> {
              return Promise.resolve(username)
            }
            `;

            export function realLogin(username: string): Promise<string> {
              return Promise.resolve(username)
            }
        "#,
        )
        .expect("failed to write ts fixture");

        let report =
            crawl_codebase(workspace.path(), &ScanPolicy::default()).expect("crawler failed");

        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("realLogin")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("fakeLogin")));
    }

    #[test]
    fn public_api_item_filters_scope_signatures_within_selected_root() {
        let workspace = TempWorkspace::new();
        let src = workspace.path().join("src");
        fs::create_dir_all(&src).expect("failed to create src");

        fs::write(
            src.join("api.ts"),
            r#"
            // @mvs-feature("auth")
            // @mvs-protocol("auth-api")
            export function login(username: string, password: string): Promise<string> {
              return Promise.resolve(`${username}:${password}`)
            }

            export interface Session {
              token: string
            }

            export const buildSession = (token: string): Session => ({ token })
        "#,
        )
        .expect("failed to write api fixture");

        let report = crawl_codebase(
            workspace.path(),
            &ScanPolicy {
                exclude_paths: Vec::new(),
                public_api_roots: vec!["src/api.ts".to_string()],
                ts_export_following: crate::mvs::manifest::TsExportFollowing::Off,
                go_export_following: crate::mvs::manifest::GoExportFollowing::Off,
                rust_export_following: crate::mvs::manifest::RustExportFollowing::Off,
                ruby_export_following: crate::mvs::manifest::RubyExportFollowing::Heuristic,
                lua_export_following: crate::mvs::manifest::LuaExportFollowing::Heuristic,
                python_export_following: crate::mvs::manifest::PythonExportFollowing::Heuristic,
                python_module_roots: Vec::new(),
                rust_workspace_members: Vec::new(),
                public_api_includes: vec!["ts/js:function login*".to_string()],
                public_api_excludes: vec!["ts/js:const buildSession".to_string()],
            },
        )
        .expect("crawler failed");

        assert!(report.feature_tags.contains("auth"));
        assert!(report.protocol_tags.contains("auth-api"));
        assert_eq!(report.public_api.len(), 1);
        assert!(report
            .public_api
            .iter()
            .any(|entry| entry.signature.starts_with("ts/js:function login")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("Session")));
        assert!(!report
            .public_api
            .iter()
            .any(|entry| entry.signature.contains("buildSession")));
    }
}
