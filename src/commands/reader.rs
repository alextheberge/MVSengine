// SPDX-License-Identifier: AGPL-3.0-only
use anyhow::Context;
use serde::Serialize;

use crate::cli::{
    OutputFormat, ValidateArgs, EXIT_MANIFEST_ERROR, EXIT_SUCCESS, EXIT_VALIDATE_INCOMPATIBLE,
};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::mvs::manifest::Manifest;
use crate::mvs::reader::validate_host_extension;

/// @mvs-feature("manifest_compatibility_validation")
/// @mvs-protocol("cli_validate_command")
pub fn run(args: ValidateArgs) -> i32 {
    match try_run(&args) {
        Ok(report) => match render_validate_report(&report, args.format) {
            Ok(()) => report.exit_code,
            Err(error) => emit_error("validate", args.format, error.exit_code, &error.message),
        },
        Err(error) => emit_error("validate", args.format, error.exit_code, &error.message),
    }
}

fn try_run(args: &ValidateArgs) -> std::result::Result<ValidateReport, CommandFailure> {
    let host = Manifest::load(&args.host_manifest)
        .with_context(|| {
            format!(
                "failed to read host manifest: {}",
                args.host_manifest.display()
            )
        })
        .map_err(|error| CommandFailure::new(EXIT_MANIFEST_ERROR, format!("{error:#}")))?;
    let extension = Manifest::load(&args.extension_manifest)
        .with_context(|| {
            format!(
                "failed to read extension manifest: {}",
                args.extension_manifest.display()
            )
        })
        .map_err(|error| CommandFailure::new(EXIT_MANIFEST_ERROR, format!("{error:#}")))?;

    let result = validate_host_extension(
        &host,
        &extension,
        args.context.as_deref(),
        args.allow_shims,
        if args.host_model_capabilities.is_empty() {
            None
        } else {
            Some(args.host_model_capabilities.as_slice())
        },
    );

    let (status, exit_code) = if result.compatible {
        if result.degraded {
            ("degraded", EXIT_SUCCESS)
        } else {
            ("ok", EXIT_SUCCESS)
        }
    } else {
        ("incompatible", EXIT_VALIDATE_INCOMPATIBLE)
    };

    Ok(ValidateReport {
        command: "validate",
        status,
        exit_code,
        compatible: result.compatible,
        degraded: result.degraded,
        host_manifest: args.host_manifest.display().to_string(),
        extension_manifest: args.extension_manifest.display().to_string(),
        target_context: args
            .context
            .clone()
            .unwrap_or_else(|| extension.identity.cont.clone()),
        reasons: result.reasons,
    })
}

fn render_validate_report(
    report: &ValidateReport,
    format: OutputFormat,
) -> std::result::Result<(), CommandFailure> {
    match format {
        OutputFormat::Text => {
            if report.compatible {
                if report.degraded {
                    println!("Compatibility: DEGRADED (legacy shim path)");
                } else {
                    println!("Compatibility: OK");
                }
            } else {
                println!("Compatibility: INCOMPATIBLE");
            }

            for reason in &report.reasons {
                println!("- {reason}");
            }
            Ok(())
        }
        OutputFormat::Json => emit_json(report),
    }
}

#[derive(Debug, Serialize)]
struct ValidateReport {
    command: &'static str,
    status: &'static str,
    exit_code: i32,
    compatible: bool,
    degraded: bool,
    host_manifest: String,
    extension_manifest: String,
    target_context: String,
    reasons: Vec<String>,
}
