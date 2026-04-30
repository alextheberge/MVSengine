// SPDX-License-Identifier: AGPL-3.0-only
//! Batch host/extension matrix validation across multiple manifests.
//!
//! Accepts a directory (or explicit list of paths) containing `mvs.json`
//! files, treats each as both a potential host and a potential extension,
//! and reports the full compatibility matrix.

use std::path::PathBuf;

use serde::Serialize;

use crate::cli::{
    OutputFormat, ValidateAllArgs, EXIT_MANIFEST_ERROR, EXIT_SUCCESS, EXIT_VALIDATE_INCOMPATIBLE,
};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::mvs::manifest::Manifest;
use crate::mvs::reader::validate_host_extension;

/// @mvs-feature("manifest_batch_validation")
/// @mvs-protocol("cli_validate_all_command")
pub fn run(args: ValidateAllArgs) -> i32 {
    let format = args.format;
    match try_run(&args) {
        Ok(report) => match render_report(&report, format) {
            Ok(()) => report.exit_code,
            Err(error) => emit_error("validate-all", format, error.exit_code, &error.message),
        },
        Err(error) => emit_error("validate-all", format, error.exit_code, &error.message),
    }
}

fn try_run(args: &ValidateAllArgs) -> Result<ValidateAllReport, CommandFailure> {
    let paths = collect_manifest_paths(args)?;

    if paths.is_empty() {
        return Err(CommandFailure::new(
            EXIT_MANIFEST_ERROR,
            "no mvs.json files found in the specified location(s)".to_string(),
        ));
    }

    // Load all manifests
    let mut loaded: Vec<(String, Manifest)> = Vec::new();
    let mut load_errors: Vec<String> = Vec::new();

    for path in &paths {
        match Manifest::load(path) {
            Ok(manifest) => loaded.push((path.display().to_string(), manifest)),
            Err(error) => {
                load_errors.push(format!("{}: {error:#}", path.display()));
            }
        }
    }

    if loaded.is_empty() {
        return Err(CommandFailure::new(
            EXIT_MANIFEST_ERROR,
            format!(
                "failed to load any manifests. Errors:\n{}",
                load_errors.join("\n")
            ),
        ));
    }

    // Build the compatibility matrix.
    // Each manifest plays both roles: if args.hosts is empty, all manifests
    // are treated as potential hosts against all extensions.
    let mut pairs: Vec<MatrixPair> = Vec::new();
    let mut any_incompatible = false;

    for (host_path, host) in &loaded {
        for (ext_path, ext) in &loaded {
            if host_path == ext_path {
                continue;
            }
            // Skip pairs that clearly aren't related (different ARCH)
            if args.same_arch_only && host.identity.arch != ext.identity.arch {
                continue;
            }

            let result =
                validate_host_extension(host, ext, args.context.as_deref(), args.allow_shims, None);

            let status = if !result.compatible {
                any_incompatible = true;
                "incompatible"
            } else if result.degraded {
                "degraded"
            } else {
                "ok"
            };

            pairs.push(MatrixPair {
                host: host_path.clone(),
                host_version: host.identity.mvs.clone(),
                extension: ext_path.clone(),
                extension_version: ext.identity.mvs.clone(),
                status,
                compatible: result.compatible,
                degraded: result.degraded,
                reasons: result.reasons,
            });
        }
    }

    let total = pairs.len();
    let incompatible_count = pairs.iter().filter(|p| !p.compatible).count();
    let degraded_count = pairs.iter().filter(|p| p.compatible && p.degraded).count();
    let ok_count = pairs.iter().filter(|p| p.compatible && !p.degraded).count();

    let compatibility_digest = CompatibilityDigest {
        incompatible: pairs
            .iter()
            .filter(|p| !p.compatible)
            .map(|p| IncompatiblePairSummary {
                host: p.host.clone(),
                extension: p.extension.clone(),
                host_version: p.host_version.clone(),
                extension_version: p.extension_version.clone(),
                reasons: p.reasons.clone(),
            })
            .collect(),
        degraded: pairs
            .iter()
            .filter(|p| p.compatible && p.degraded)
            .map(|p| DegradedPairSummary {
                host: p.host.clone(),
                extension: p.extension.clone(),
                host_version: p.host_version.clone(),
                extension_version: p.extension_version.clone(),
            })
            .collect(),
    };

    Ok(ValidateAllReport {
        command: "validate-all",
        exit_code: if any_incompatible {
            EXIT_VALIDATE_INCOMPATIBLE
        } else {
            EXIT_SUCCESS
        },
        manifests_loaded: loaded.len(),
        load_errors,
        total_pairs: total,
        ok_count,
        degraded_count,
        incompatible_count,
        compatibility: compatibility_digest,
        pairs,
    })
}

fn collect_manifest_paths(args: &ValidateAllArgs) -> Result<Vec<PathBuf>, CommandFailure> {
    // If explicit manifest paths were given, use them directly
    if !args.manifests.is_empty() {
        return Ok(args.manifests.clone());
    }

    // Otherwise, search the directory for mvs.json files
    let dir = args.dir.as_deref().unwrap_or(std::path::Path::new("."));
    let mut paths = Vec::new();
    collect_manifests_in_dir(dir, &mut paths, 0, args.max_depth);
    Ok(paths)
}

fn collect_manifests_in_dir(
    dir: &std::path::Path,
    paths: &mut Vec<PathBuf>,
    depth: usize,
    max_depth: usize,
) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some("mvs.json") {
            paths.push(path);
        } else if path.is_dir() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.starts_with('.') && name_str != "target" && name_str != "node_modules" {
                collect_manifests_in_dir(&path, paths, depth + 1, max_depth);
            }
        }
    }
}

fn render_report(report: &ValidateAllReport, format: OutputFormat) -> Result<(), CommandFailure> {
    match format {
        OutputFormat::Text => {
            println!(
                "Validated {} manifest pair(s) across {} loaded manifest(s).",
                report.total_pairs, report.manifests_loaded
            );
            println!(
                "  OK: {}  Degraded: {}  Incompatible: {}",
                report.ok_count, report.degraded_count, report.incompatible_count
            );
            if !report.load_errors.is_empty() {
                println!("\nLoad errors ({}):", report.load_errors.len());
                for error in &report.load_errors {
                    println!("  - {error}");
                }
            }
            if report.incompatible_count > 0 {
                println!("\nIncompatible pairs:");
                for pair in report.pairs.iter().filter(|p| !p.compatible) {
                    println!(
                        "  HOST {}  ({})  <->  EXT {}  ({})",
                        pair.host_version, pair.host, pair.extension_version, pair.extension
                    );
                    for reason in &pair.reasons {
                        println!("    - {reason}");
                    }
                }
            }
            if report.degraded_count > 0 {
                println!("\nDegraded pairs (shim path):");
                for pair in report.pairs.iter().filter(|p| p.compatible && p.degraded) {
                    println!(
                        "  HOST {}  <->  EXT {}",
                        pair.host_version, pair.extension_version
                    );
                }
            }
            println!(
                "\nMachine-readable compatibility summary: use `--format json` (see `compatibility` object)."
            );
            Ok(())
        }
        OutputFormat::Json => emit_json(report),
    }
}

#[derive(Debug, Serialize)]
struct ValidateAllReport {
    command: &'static str,
    exit_code: i32,
    manifests_loaded: usize,
    load_errors: Vec<String>,
    total_pairs: usize,
    ok_count: usize,
    degraded_count: usize,
    incompatible_count: usize,
    /// Short compatibility lists for automation (subset of `pairs`).
    compatibility: CompatibilityDigest,
    pairs: Vec<MatrixPair>,
}

#[derive(Debug, Serialize)]
struct CompatibilityDigest {
    incompatible: Vec<IncompatiblePairSummary>,
    degraded: Vec<DegradedPairSummary>,
}

#[derive(Debug, Serialize)]
struct IncompatiblePairSummary {
    host: String,
    extension: String,
    host_version: String,
    extension_version: String,
    reasons: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DegradedPairSummary {
    host: String,
    extension: String,
    host_version: String,
    extension_version: String,
}

#[derive(Debug, Serialize)]
struct MatrixPair {
    host: String,
    host_version: String,
    extension: String,
    extension_version: String,
    status: &'static str,
    compatible: bool,
    degraded: bool,
    reasons: Vec<String>,
}
