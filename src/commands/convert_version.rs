// SPDX-License-Identifier: AGPL-3.0-only
use serde::Serialize;

use crate::cli::{ConvertVersionArgs, OutputFormat, EXIT_SCHEME_ERROR, EXIT_SUCCESS};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::mvs::manifest::Identity;
use crate::mvs::schemes::{self, AxisMapping, VersionScheme};

/// @mvs-feature("legacy_version_conversion")
/// @mvs-protocol("cli_convert_version_command")
pub fn run(args: ConvertVersionArgs) -> i32 {
    match try_run(&args) {
        Ok(report) => match render_report(&report, args.format) {
            Ok(()) => report.exit_code,
            Err(error) => emit_error(
                "convert-version",
                args.format,
                error.exit_code,
                &error.message,
            ),
        },
        Err(error) => emit_error(
            "convert-version",
            args.format,
            error.exit_code,
            &error.message,
        ),
    }
}

fn try_run(args: &ConvertVersionArgs) -> Result<ConvertVersionReport, CommandFailure> {
    let forced_scheme = match &args.scheme {
        Some(name) => Some(VersionScheme::parse_name(name).ok_or_else(|| {
            CommandFailure::new(
                EXIT_SCHEME_ERROR,
                format!(
                    "unknown --scheme `{name}`; expected one of semver, zerover, pep440, \
                     maven-gradle, dotnet4, go-modules, calver, integer, debian-rpm, custom"
                ),
            )
        })?),
        None => None,
    };

    let legacy = match forced_scheme {
        Some(scheme) => schemes::parse(&args.version, scheme, args.scheme_regex.as_deref())
            .map_err(|error| CommandFailure::new(EXIT_SCHEME_ERROR, format!("{error:#}")))?,
        None => schemes::parse_auto(&args.version)
            .map_err(|error| CommandFailure::new(EXIT_SCHEME_ERROR, format!("{error:#}")))?,
    };

    let (mapping, mapping_source): (AxisMapping, &'static str) = match &args.map {
        Some(spec) => (
            AxisMapping::parse_spec(spec)
                .map_err(|error| CommandFailure::new(EXIT_SCHEME_ERROR, format!("{error:#}")))?,
            "custom",
        ),
        None => (schemes::default_mapping_for(legacy.scheme), "default"),
    };

    let mut axes = schemes::resolve_axes(&legacy, &mapping);
    if let Some(prot) = args.prot {
        axes.prot = prot;
    }

    let identity = Identity::format_mvs(axes.arch, axes.feat, axes.prot, axes.fix, &args.context);
    let semver_projection = Identity::semver_projection(axes.arch, axes.feat, axes.fix);

    Ok(ConvertVersionReport {
        command: "convert-version",
        status: "converted",
        exit_code: EXIT_SUCCESS,
        input: args.version.clone(),
        scheme: legacy.scheme.as_str(),
        scheme_forced: forced_scheme.is_some(),
        components: legacy
            .components
            .iter()
            .map(|(name, value)| ComponentValue {
                name: name.clone(),
                value: *value,
            })
            .collect(),
        epoch: legacy.epoch,
        prerelease: legacy.prerelease.clone(),
        build_metadata: legacy.build_metadata.clone(),
        revision: legacy.revision.clone(),
        mapping_source,
        axes: AxesPayload {
            arch: axes.arch,
            feat: axes.feat,
            prot: axes.prot,
            fix: axes.fix,
        },
        identity,
        semver_projection,
        advisories: schemes::advisories(&legacy, &mapping),
    })
}

fn render_report(
    report: &ConvertVersionReport,
    format: OutputFormat,
) -> std::result::Result<(), CommandFailure> {
    match format {
        OutputFormat::Text => {
            let detection = if report.scheme_forced {
                "forced"
            } else {
                "auto-detected"
            };
            println!(
                "\"{}\" {} as {} (mapping: {})",
                report.input, detection, report.scheme, report.mapping_source
            );
            for component in &report.components {
                println!("  {} = {}", component.name, component.value);
            }
            if let Some(epoch) = report.epoch {
                println!("  epoch = {epoch}");
            }
            if let Some(prerelease) = &report.prerelease {
                println!("  prerelease: {prerelease}");
            }
            if let Some(build_metadata) = &report.build_metadata {
                println!("  build metadata: {build_metadata}");
            }
            if let Some(revision) = &report.revision {
                println!("  packaging revision: {revision}");
            }
            println!(
                "MVS identity: {} (ARCH {}, FEAT {}, PROT {}, FIX {})",
                report.identity,
                report.axes.arch,
                report.axes.feat,
                report.axes.prot,
                report.axes.fix
            );
            println!("SemVer projection: {}", report.semver_projection);
            for advisory in &report.advisories {
                println!("- {advisory}");
            }
            Ok(())
        }
        OutputFormat::Json => emit_json(report).map_err(|error| {
            CommandFailure::new(
                EXIT_SCHEME_ERROR,
                format!("failed to render convert-version output: {}", error.message),
            )
        }),
    }
}

#[derive(Debug, Serialize)]
struct ConvertVersionReport {
    command: &'static str,
    status: &'static str,
    exit_code: i32,
    input: String,
    scheme: &'static str,
    scheme_forced: bool,
    components: Vec<ComponentValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    epoch: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prerelease: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    build_metadata: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    mapping_source: &'static str,
    axes: AxesPayload,
    identity: String,
    semver_projection: String,
    advisories: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ComponentValue {
    name: String,
    value: u64,
}

#[derive(Debug, Serialize)]
struct AxesPayload {
    arch: u64,
    feat: u64,
    prot: u64,
    fix: u64,
}
