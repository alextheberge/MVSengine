// SPDX-License-Identifier: AGPL-3.0-only
use serde::Serialize;

use crate::cli::{OutputFormat, SelfUpdateArgs, EXIT_SUCCESS, EXIT_UPDATE_ERROR};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::update::{self, CheckStatus};

/// @mvs-feature("manager_self_update")
/// @mvs-protocol("cli_self_update_command")
pub fn run(args: SelfUpdateArgs) -> i32 {
    match try_run(&args) {
        Ok(report) => match render_report(&report, args.format) {
            Ok(()) => report.exit_code,
            Err(error) => emit_error("self-update", args.format, error.exit_code, &error.message),
        },
        Err(error) => emit_error("self-update", args.format, error.exit_code, &error.message),
    }
}

fn try_run(args: &SelfUpdateArgs) -> Result<SelfUpdateReport, CommandFailure> {
    let current_version = update::current_version().to_string();

    if args.check {
        return match update::check_for_update() {
            Ok(CheckStatus::UpToDate) => Ok(SelfUpdateReport {
                command: "self-update",
                status: "up_to_date",
                exit_code: EXIT_SUCCESS,
                mode: "check",
                current_version: current_version.clone(),
                latest_version: current_version,
                release_url: None,
            }),
            Ok(CheckStatus::UpdateAvailable(release)) => Ok(SelfUpdateReport {
                command: "self-update",
                status: "update_available",
                exit_code: EXIT_SUCCESS,
                mode: "check",
                current_version,
                latest_version: release.version,
                release_url: Some(release.release_url),
            }),
            Err(error) => Err(CommandFailure::new(
                EXIT_UPDATE_ERROR,
                format!("failed to check for updates: {error:#}"),
            )),
        };
    }

    match update::perform_self_update() {
        Ok(release) => {
            let updated = release.version != current_version;
            Ok(SelfUpdateReport {
                command: "self-update",
                status: if updated { "updated" } else { "up_to_date" },
                exit_code: EXIT_SUCCESS,
                mode: "update",
                current_version,
                latest_version: release.version,
                release_url: Some(release.release_url),
            })
        }
        Err(error) => Err(CommandFailure::new(
            EXIT_UPDATE_ERROR,
            format!("failed to update mvs-manager: {error:#}"),
        )),
    }
}

fn render_report(report: &SelfUpdateReport, format: OutputFormat) -> Result<(), CommandFailure> {
    match format {
        OutputFormat::Text => {
            match report.status {
                "up_to_date" => {
                    println!("mvs-manager is up to date (v{}).", report.current_version);
                }
                "update_available" => {
                    println!(
                        "Update available: v{} -> v{}.",
                        report.current_version, report.latest_version
                    );
                    println!("Run `mvs-manager self-update` to install it.");
                }
                "updated" => {
                    println!(
                        "Updated mvs-manager from v{} to v{}.",
                        report.current_version, report.latest_version
                    );
                }
                _ => {}
            }

            if let Some(release_url) = report.release_url.as_deref() {
                println!("Release: {release_url}");
            }
            Ok(())
        }
        OutputFormat::Json => emit_json(report),
    }
}

#[derive(Debug, Serialize)]
struct SelfUpdateReport {
    command: &'static str,
    status: &'static str,
    exit_code: i32,
    mode: &'static str,
    current_version: String,
    latest_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    release_url: Option<String>,
}
