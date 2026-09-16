// SPDX-License-Identifier: AGPL-3.0-only
use anyhow::Context;
use serde::Serialize;

use crate::cli::{
    OutputFormat, SyncArgs, EXIT_MANIFEST_ERROR, EXIT_SUCCESS, EXIT_SYNC_DRIFT, EXIT_SYNC_ERROR,
};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::mvs::manifest::{Manifest, VersionFileEntry};
use crate::mvs::version_sources;

/// @mvs-feature("version_file_sync")
/// @mvs-protocol("cli_sync_command")
pub fn run(args: SyncArgs) -> i32 {
    match try_run(&args) {
        Ok(report) => match render_report(&report, args.format) {
            Ok(()) => report.exit_code,
            Err(error) => emit_error("sync", args.format, error.exit_code, &error.message),
        },
        Err(error) => emit_error("sync", args.format, error.exit_code, &error.message),
    }
}

fn try_run(args: &SyncArgs) -> Result<SyncReport, CommandFailure> {
    let manifest_path = args.root.join(&args.manifest);
    let manifest = Manifest::load(&manifest_path)
        .with_context(|| format!("failed to read manifest: {}", manifest_path.display()))
        .map_err(|error| CommandFailure::new(EXIT_MANIFEST_ERROR, format!("{error:#}")))?;

    let detected = manifest.release.version_files.is_empty();
    let entries: Vec<VersionFileEntry> = if detected {
        version_sources::detect(&args.root)
    } else {
        manifest.release.version_files.clone()
    };

    let mode = if args.check {
        "check"
    } else if args.dry_run {
        "dry_run"
    } else {
        "apply"
    };

    if entries.is_empty() {
        return Ok(SyncReport {
            command: "sync",
            status: "no_files",
            exit_code: EXIT_SUCCESS,
            manifest: manifest_path.display().to_string(),
            root: args.root.display().to_string(),
            mode,
            detected,
            saved_detected: false,
            files: Vec::new(),
        });
    }

    let suffix = args.suffix.as_deref().filter(|s| !s.is_empty());
    let mut files = Vec::with_capacity(entries.len());
    let mut has_error = false;
    let mut has_drift = false;
    let mut has_change = false;

    for entry in &entries {
        let mut projected = version_sources::projected_version(entry, &manifest.identity);
        if let Some(suffix) = suffix {
            projected = format!("{projected}-{suffix}");
        }

        let current = match version_sources::read_version(&args.root, entry) {
            Ok(value) => value,
            Err(error) => {
                has_error = true;
                files.push(SyncFileResult {
                    path: entry.path.clone(),
                    kind: entry.kind.as_str().to_string(),
                    projection: entry.projection.as_str(),
                    projected_version: projected,
                    current_version: None,
                    status: "error",
                    error: Some(format!("{error:#}")),
                });
                continue;
            }
        };

        if current == projected {
            files.push(SyncFileResult {
                path: entry.path.clone(),
                kind: entry.kind.as_str().to_string(),
                projection: entry.projection.as_str(),
                projected_version: projected,
                current_version: Some(current),
                status: "in_sync",
                error: None,
            });
            continue;
        }

        if args.check || args.dry_run {
            has_drift = true;
            files.push(SyncFileResult {
                path: entry.path.clone(),
                kind: entry.kind.as_str().to_string(),
                projection: entry.projection.as_str(),
                projected_version: projected,
                current_version: Some(current),
                status: if args.check { "drift" } else { "would_update" },
                error: None,
            });
            continue;
        }

        match version_sources::write_version(&args.root, entry, &projected) {
            Ok(_) => {
                has_change = true;
                files.push(SyncFileResult {
                    path: entry.path.clone(),
                    kind: entry.kind.as_str().to_string(),
                    projection: entry.projection.as_str(),
                    projected_version: projected,
                    current_version: Some(current),
                    status: "updated",
                    error: None,
                });
            }
            Err(error) => {
                has_error = true;
                files.push(SyncFileResult {
                    path: entry.path.clone(),
                    kind: entry.kind.as_str().to_string(),
                    projection: entry.projection.as_str(),
                    projected_version: projected,
                    current_version: Some(current),
                    status: "error",
                    error: Some(format!("{error:#}")),
                });
            }
        }
    }

    let mut saved_detected = false;
    if detected && args.save_detected && !args.check && !args.dry_run && !has_error {
        let mut updated_manifest = manifest.clone();
        updated_manifest.release.version_files = entries.clone();
        updated_manifest
            .write(&manifest_path)
            .with_context(|| {
                format!(
                    "failed to persist detected version files to `{}`",
                    manifest_path.display()
                )
            })
            .map_err(|error| CommandFailure::new(EXIT_MANIFEST_ERROR, format!("{error:#}")))?;
        saved_detected = true;
    }

    let exit_code = if has_error {
        EXIT_SYNC_ERROR
    } else if args.check && has_drift {
        EXIT_SYNC_DRIFT
    } else {
        EXIT_SUCCESS
    };

    let status = if has_error {
        "error"
    } else if args.check && has_drift {
        "drift"
    } else if args.dry_run && has_drift {
        "would_update"
    } else if has_change {
        "updated"
    } else {
        "in_sync"
    };

    Ok(SyncReport {
        command: "sync",
        status,
        exit_code,
        manifest: manifest_path.display().to_string(),
        root: args.root.display().to_string(),
        mode,
        detected,
        saved_detected,
        files,
    })
}

fn render_report(
    report: &SyncReport,
    format: OutputFormat,
) -> std::result::Result<(), CommandFailure> {
    match format {
        OutputFormat::Text => {
            if report.status == "no_files" {
                println!(
                    "sync: no known version files declared in release.version_files or auto-detected under `{}`.",
                    report.root
                );
                return Ok(());
            }

            if report.detected {
                println!("sync: release.version_files is empty; using auto-detected files.");
            }

            for file in &report.files {
                let marker = match file.status {
                    "in_sync" => "=",
                    "updated" => "->",
                    "would_update" => "~>",
                    "drift" => "!=",
                    _ => "x",
                };
                match file.status {
                    "error" => println!(
                        "  x {} ({}): {}",
                        file.path,
                        file.kind,
                        file.error.as_deref().unwrap_or("unknown error")
                    ),
                    "in_sync" => println!(
                        "  {marker} {} ({}): {}",
                        file.path, file.kind, file.projected_version
                    ),
                    _ => println!(
                        "  {marker} {} ({}): {} -> {}",
                        file.path,
                        file.kind,
                        file.current_version.as_deref().unwrap_or("?"),
                        file.projected_version
                    ),
                }
            }

            if report.saved_detected {
                println!("sync: persisted auto-detected version files into the manifest.");
            }

            let summary = match report.status {
                "in_sync" => "all version files already match the manifest projection.",
                "updated" => "version files updated to match the manifest projection.",
                "would_update" => {
                    "would update version files to match the manifest projection (dry run)."
                }
                "drift" => "version files are out of sync with the manifest projection.",
                "error" => "one or more version files could not be read or written.",
                _ => "sync finished.",
            };
            println!("sync: {summary}");

            Ok(())
        }
        OutputFormat::Json => emit_json(report).map_err(|error| {
            CommandFailure::new(
                EXIT_SYNC_ERROR,
                format!("failed to render sync output: {}", error.message),
            )
        }),
    }
}

#[derive(Debug, Serialize)]
struct SyncReport {
    command: &'static str,
    status: &'static str,
    exit_code: i32,
    manifest: String,
    root: String,
    mode: &'static str,
    detected: bool,
    saved_detected: bool,
    files: Vec<SyncFileResult>,
}

#[derive(Debug, Serialize)]
struct SyncFileResult {
    path: String,
    kind: String,
    projection: &'static str,
    projected_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_version: Option<String>,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}
