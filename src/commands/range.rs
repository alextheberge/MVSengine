// SPDX-License-Identifier: AGPL-3.0-only
use anyhow::Context;
use serde::Serialize;

use crate::cli::{OutputFormat, RangeArgs, EXIT_MANIFEST_ERROR, EXIT_RANGE_ERROR, EXIT_SUCCESS};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::mvs::manifest::Manifest;
use crate::mvs::range::{self, Ecosystem};

/// @mvs-feature("dependency_range_translation")
/// @mvs-protocol("cli_range_command")
pub fn run(args: RangeArgs) -> i32 {
    match try_run(&args) {
        Ok(report) => match render_report(&report, args.format) {
            Ok(()) => EXIT_SUCCESS,
            Err(error) => emit_error("range", args.format, error.exit_code, &error.message),
        },
        Err(error) => emit_error("range", args.format, error.exit_code, &error.message),
    }
}

fn try_run(args: &RangeArgs) -> Result<RangeReport, CommandFailure> {
    let manifest = Manifest::load(&args.manifest)
        .with_context(|| format!("failed to load manifest: {}", args.manifest.display()))
        .map_err(|error| CommandFailure::new(EXIT_MANIFEST_ERROR, format!("{error:#}")))?;

    let (min_prot, max_prot, source) = resolve_bounds(args, &manifest)?;

    let ecosystems: Vec<Ecosystem> = args
        .for_ecosystems
        .iter()
        .map(|name| {
            Ecosystem::parse_name(name).ok_or_else(|| {
                CommandFailure::new(
                    EXIT_RANGE_ERROR,
                    format!(
                        "unknown --for ecosystem `{name}`; expected one of npm, cargo, pip, maven"
                    ),
                )
            })
        })
        .collect::<Result<_, _>>()?;

    let resolved = range::resolve_range(&manifest, min_prot, max_prot)
        .map_err(|error| CommandFailure::new(EXIT_RANGE_ERROR, format!("{error:#}")))?;

    let constraints = ecosystems
        .iter()
        .map(|ecosystem| ConstraintPayload {
            ecosystem: ecosystem.as_str(),
            constraint: range::format_for_ecosystem(*ecosystem, &resolved),
        })
        .collect();

    Ok(RangeReport {
        command: "range",
        manifest: args.manifest.display().to_string(),
        source,
        min_prot: resolved.min_prot,
        max_prot: resolved.max_prot,
        arch: resolved.arch,
        lower_bound: resolved.lower_bound,
        upper_bound: resolved.upper_bound,
        matched_versions: resolved.matched_versions,
        constraints,
    })
}

fn resolve_bounds(
    args: &RangeArgs,
    manifest: &Manifest,
) -> Result<(u64, u64, &'static str), CommandFailure> {
    let explicit = args.min_prot.is_some() || args.max_prot.is_some();
    if explicit && (args.host || args.extension) {
        return Err(CommandFailure::new(
            EXIT_RANGE_ERROR,
            "pass either --min-prot/--max-prot or --host/--extension, not both".to_string(),
        ));
    }
    if args.host && args.extension {
        return Err(CommandFailure::new(
            EXIT_RANGE_ERROR,
            "pass only one of --host or --extension".to_string(),
        ));
    }

    if args.host {
        let range = &manifest.compatibility.host_range;
        return Ok((range.min_prot, range.max_prot, "compatibility.host_range"));
    }
    if args.extension {
        let range = &manifest.compatibility.extension_range;
        return Ok((
            range.min_prot,
            range.max_prot,
            "compatibility.extension_range",
        ));
    }

    match (args.min_prot, args.max_prot) {
        (Some(min), Some(max)) => Ok((min, max, "--min-prot/--max-prot")),
        _ => Err(CommandFailure::new(
            EXIT_RANGE_ERROR,
            "pass --min-prot and --max-prot together, or --host, or --extension".to_string(),
        )),
    }
}

fn render_report(
    report: &RangeReport,
    format: OutputFormat,
) -> std::result::Result<(), CommandFailure> {
    match format {
        OutputFormat::Text => {
            println!(
                "range: PROT [{}, {}] (ARCH {}, from {}) -> {} .. {}",
                report.min_prot,
                report.max_prot,
                report.arch,
                report.source,
                report.lower_bound,
                report.upper_bound.as_deref().unwrap_or("(open)")
            );
            for constraint in &report.constraints {
                println!("  {}: {}", constraint.ecosystem, constraint.constraint);
            }
            Ok(())
        }
        OutputFormat::Json => emit_json(report).map_err(|error| {
            CommandFailure::new(
                EXIT_RANGE_ERROR,
                format!("failed to render range output: {}", error.message),
            )
        }),
    }
}

#[derive(Debug, Serialize)]
struct RangeReport {
    command: &'static str,
    manifest: String,
    source: &'static str,
    min_prot: u64,
    max_prot: u64,
    arch: u64,
    lower_bound: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    upper_bound: Option<String>,
    matched_versions: usize,
    constraints: Vec<ConstraintPayload>,
}

#[derive(Debug, Serialize)]
struct ConstraintPayload {
    ecosystem: &'static str,
    constraint: String,
}
