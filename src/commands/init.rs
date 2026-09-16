// SPDX-License-Identifier: AGPL-3.0-only
use anyhow::Context;

use crate::cli::{InitArgs, OutputFormat, EXIT_GENERATE_ERROR, EXIT_INIT_ERROR, EXIT_SUCCESS};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::mvs::manifest::Manifest;
use crate::mvs::project_detect::{build_scan_policy, detect_project, infer_context};

/// @mvs-feature("manifest_init")
/// @mvs-protocol("cli_init_command")
pub fn run(args: InitArgs) -> i32 {
    match try_run(&args) {
        Ok(report) => match render_init_report(&report, args.format) {
            Ok(()) => report.exit_code,
            Err(error) => emit_error("init", args.format, error.exit_code, &error.message),
        },
        Err(error) => emit_error("init", args.format, error.exit_code, &error.message),
    }
}

fn try_run(args: &InitArgs) -> Result<InitReport, CommandFailure> {
    let manifest_path = args.root.join(&args.manifest);

    if manifest_path.exists() && !args.force {
        return Err(CommandFailure::new(
            EXIT_INIT_ERROR,
            format!(
                "manifest already exists at `{}`. Use --force to overwrite.",
                manifest_path.display()
            ),
        ));
    }

    let detected = detect_project(&args.root).map_err(|error| {
        CommandFailure::new(
            EXIT_GENERATE_ERROR,
            format!("project detection failed: {error:#}"),
        )
    })?;

    let context = args
        .context
        .as_deref()
        .unwrap_or_else(|| infer_context(&detected));

    let scan_policy = build_scan_policy(&args.root, &detected, args.preset.as_deref());

    let mut manifest = Manifest::default_for_context(context);
    manifest.scan_policy = scan_policy;

    if args.dry_run {
        let preview = serde_json::to_string_pretty(&manifest).map_err(|e| {
            CommandFailure::new(EXIT_GENERATE_ERROR, format!("serialization failed: {e}"))
        })?;
        return Ok(InitReport {
            exit_code: EXIT_SUCCESS,
            manifest_path: manifest_path.display().to_string(),
            dry_run: true,
            detected_languages: detected.languages.into_iter().collect(),
            detected_markers: detected.markers,
            context: context.to_string(),
            preview: Some(preview),
        });
    }

    manifest
        .write(&manifest_path)
        .with_context(|| format!("failed to write manifest to `{}`", manifest_path.display()))
        .map_err(|e| CommandFailure::new(EXIT_GENERATE_ERROR, format!("{e:#}")))?;

    Ok(InitReport {
        exit_code: EXIT_SUCCESS,
        manifest_path: manifest_path.display().to_string(),
        dry_run: false,
        detected_languages: detected.languages.into_iter().collect(),
        detected_markers: detected.markers,
        context: context.to_string(),
        preview: None,
    })
}

// ── Report ───────────────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
struct InitReport {
    exit_code: i32,
    manifest_path: String,
    dry_run: bool,
    detected_languages: Vec<&'static str>,
    detected_markers: Vec<String>,
    context: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview: Option<String>,
}

fn render_init_report(report: &InitReport, format: OutputFormat) -> Result<(), CommandFailure> {
    match format {
        OutputFormat::Text => {
            if report.dry_run {
                println!(
                    "Dry run — manifest would be written to: {}",
                    report.manifest_path
                );
            } else {
                println!("Initialized manifest at: {}", report.manifest_path);
            }
            if !report.detected_languages.is_empty() {
                println!(
                    "Detected languages: {}",
                    report.detected_languages.join(", ")
                );
            }
            if !report.detected_markers.is_empty() {
                println!(
                    "Project markers found: {}",
                    report.detected_markers.join(", ")
                );
            }
            println!("Context: {}", report.context);
            if report.dry_run {
                if let Some(preview) = &report.preview {
                    println!("\n--- mvs.json preview ---\n{preview}");
                }
            } else {
                println!(
                    "Run `mvs-manager generate` to scan the codebase and populate evidence hashes."
                );
            }
            Ok(())
        }
        OutputFormat::Json => emit_json(report),
    }
}
