// SPDX-License-Identifier: AGPL-3.0-only
//! Suggests the tightest valid `host_range` / `extension_range` values given
//! two manifests, so users don't have to hand-compute compatible protocol ranges.

use serde::Serialize;

use crate::cli::{ConstraintArgs, OutputFormat, EXIT_MANIFEST_ERROR, EXIT_SUCCESS};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::mvs::manifest::{Manifest, ProtocolRange};
use crate::mvs::reader::validate_host_extension;

/// @mvs-feature("constraint_suggestion")
/// @mvs-protocol("cli_constraint_command")
pub fn run(args: ConstraintArgs) -> i32 {
    let format = args.format;
    match try_run(&args) {
        Ok(report) => match render_report(&report, format) {
            Ok(()) => report.exit_code,
            Err(error) => emit_error("constraint", format, error.exit_code, &error.message),
        },
        Err(error) => emit_error("constraint", format, error.exit_code, &error.message),
    }
}

fn try_run(args: &ConstraintArgs) -> Result<ConstraintReport, CommandFailure> {
    let host = Manifest::load(&args.host_manifest).map_err(|e| {
        CommandFailure::new(
            EXIT_MANIFEST_ERROR,
            format!(
                "failed to load host manifest `{}`: {e:#}",
                args.host_manifest.display()
            ),
        )
    })?;
    let ext = Manifest::load(&args.extension_manifest).map_err(|e| {
        CommandFailure::new(
            EXIT_MANIFEST_ERROR,
            format!(
                "failed to load extension manifest `{}`: {e:#}",
                args.extension_manifest.display()
            ),
        )
    })?;

    let host_prot = host.identity.prot;
    let ext_prot = ext.identity.prot;
    let la = args.lookahead;

    // Tightest ranges that make both sides pass validation:
    //
    //   host.extension_range  must contain ext_prot
    //   ext.host_range        must contain host_prot
    //
    // With lookahead=0 that's a single-value range on each side.
    // With lookahead=N we extend each bound outward by N.

    let suggested_host_extension_range = ProtocolRange {
        min_prot: ext_prot.saturating_sub(la),
        max_prot: ext_prot + la,
    };
    let suggested_ext_host_range = ProtocolRange {
        min_prot: host_prot.saturating_sub(la),
        max_prot: host_prot + la,
    };

    // Verify the suggestion actually passes validation
    let mut probe_host = host.clone();
    let mut probe_ext = ext.clone();
    probe_host.compatibility.extension_range = suggested_host_extension_range.clone();
    probe_ext.compatibility.host_range = suggested_ext_host_range.clone();

    let result = validate_host_extension(&probe_host, &probe_ext, None, false, None);

    // Current ranges
    let current_compatible = validate_host_extension(&host, &ext, None, false, None).compatible;

    Ok(ConstraintReport {
        exit_code: EXIT_SUCCESS,
        host_manifest: args.host_manifest.display().to_string(),
        host_version: host.identity.mvs.clone(),
        host_prot,
        extension_manifest: args.extension_manifest.display().to_string(),
        extension_version: ext.identity.mvs.clone(),
        extension_prot: ext_prot,
        lookahead: la,
        currently_compatible: current_compatible,
        suggested_host_extension_range,
        suggested_extension_host_range: suggested_ext_host_range,
        suggestion_valid: result.compatible,
        current_host_extension_range: host.compatibility.extension_range.clone(),
        current_extension_host_range: ext.compatibility.host_range.clone(),
    })
}

fn render_report(report: &ConstraintReport, format: OutputFormat) -> Result<(), CommandFailure> {
    match format {
        OutputFormat::Text => {
            println!(
                "Host:      {} (PROT {})",
                report.host_version, report.host_prot
            );
            println!(
                "Extension: {} (PROT {})",
                report.extension_version, report.extension_prot
            );
            println!();
            println!(
                "Current compatibility: {}",
                if report.currently_compatible {
                    "COMPATIBLE"
                } else {
                    "INCOMPATIBLE"
                }
            );
            println!();
            println!("Suggested ranges (lookahead={}):", report.lookahead);
            println!(
                "  In host manifest     — compatibility.extension_range: {{ min_prot: {}, max_prot: {} }}",
                report.suggested_host_extension_range.min_prot,
                report.suggested_host_extension_range.max_prot
            );
            println!(
                "  In extension manifest — compatibility.host_range:      {{ min_prot: {}, max_prot: {} }}",
                report.suggested_extension_host_range.min_prot,
                report.suggested_extension_host_range.max_prot
            );
            println!(
                "  Suggestion passes validation: {}",
                if report.suggestion_valid {
                    "yes"
                } else {
                    "no (check ARCH mismatch or other axes)"
                }
            );

            if report.current_host_extension_range != report.suggested_host_extension_range
                || report.current_extension_host_range != report.suggested_extension_host_range
            {
                println!();
                println!("Current ranges for reference:");
                println!(
                    "  host.extension_range:     {{ min_prot: {}, max_prot: {} }}",
                    report.current_host_extension_range.min_prot,
                    report.current_host_extension_range.max_prot
                );
                println!(
                    "  extension.host_range:     {{ min_prot: {}, max_prot: {} }}",
                    report.current_extension_host_range.min_prot,
                    report.current_extension_host_range.max_prot
                );
            }
            Ok(())
        }
        OutputFormat::Json => emit_json(report),
    }
}

#[derive(Debug, Serialize)]
struct ConstraintReport {
    exit_code: i32,
    host_manifest: String,
    host_version: String,
    host_prot: u64,
    extension_manifest: String,
    extension_version: String,
    extension_prot: u64,
    lookahead: u64,
    currently_compatible: bool,
    suggested_host_extension_range: ProtocolRange,
    suggested_extension_host_range: ProtocolRange,
    suggestion_valid: bool,
    current_host_extension_range: ProtocolRange,
    current_extension_host_range: ProtocolRange,
}
