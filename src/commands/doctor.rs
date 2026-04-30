// SPDX-License-Identifier: AGPL-3.0-only
//! Environment and install diagnostics for troubleshooting CI and local setups.

use serde::Serialize;

use crate::cli::{DoctorArgs, OutputFormat, EXIT_OUTPUT_ERROR, EXIT_SUCCESS};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::update::{
    self, github_latest_release_api_url, github_token_configured, install_ps1_raw_url,
    install_sh_raw_url, path_matches_primary_install, repo_slug, self_update_block_reason,
    which_mvs_manager, ALLOW_UNSAFE_SELF_UPDATE_ENV, MVS_REPO_ENV, MVS_UPDATE_REPO_ENV,
    UPDATE_DISABLE_ENV, UPDATE_GITHUB_TOKEN_ENV,
};

/// @mvs-feature("manager_doctor")
/// @mvs-protocol("cli_doctor_command")
pub fn run(args: DoctorArgs) -> i32 {
    match try_run(&args) {
        Ok(report) => match render(&report, args.format) {
            Ok(()) => EXIT_SUCCESS,
            Err(error) => emit_error("doctor", args.format, error.exit_code, &error.message),
        },
        Err(error) => emit_error("doctor", args.format, error.exit_code, &error.message),
    }
}

fn try_run(args: &DoctorArgs) -> Result<DoctorReport, CommandFailure> {
    let current_exe = std::env::current_exe().map_err(|e| {
        CommandFailure::new(
            EXIT_OUTPUT_ERROR,
            format!("failed to resolve current executable: {e}"),
        )
    })?;

    let path_which = which_mvs_manager();
    let path_on_path_matches_exe = path_matches_primary_install();

    let mut warnings = Vec::new();
    if let Some(reason) = self_update_block_reason() {
        warnings.push(format!(
            "self-update would be refused from this binary ({reason})"
        ));
    }
    if path_which.is_some() && !path_on_path_matches_exe {
        warnings.push(
            "`mvs-manager` on PATH differs from the running executable — check for multiple installs."
                .to_string(),
        );
    }

    let manifest_path = args.root.join(&args.manifest);
    let manifest_present = manifest_path.exists();
    let manifest_readable =
        manifest_path.is_file() && std::fs::read_to_string(&manifest_path).is_ok();

    let update_state_path = update::default_state_file_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(unknown)".to_string());

    let github_api = github_latest_release_api_url().map_err(|e| {
        CommandFailure::new(EXIT_OUTPUT_ERROR, format!("invalid {MVS_REPO_ENV}: {e:#}"))
    })?;
    let install_sh = install_sh_raw_url().map_err(|e| {
        CommandFailure::new(EXIT_OUTPUT_ERROR, format!("invalid {MVS_REPO_ENV}: {e:#}"))
    })?;
    let install_ps1 = install_ps1_raw_url().map_err(|e| {
        CommandFailure::new(EXIT_OUTPUT_ERROR, format!("invalid {MVS_REPO_ENV}: {e:#}"))
    })?;

    Ok(DoctorReport {
        command: "doctor",
        version: update::current_version().to_string(),
        current_exe: current_exe.display().to_string(),
        path_which: path_which.as_ref().map(|p| p.display().to_string()),
        path_on_path_matches_exe,
        repo_slug: repo_slug(),
        github_latest_release_url: github_api,
        install_sh_url: install_sh,
        install_ps1_url: install_ps1,
        manifest_path: manifest_path.display().to_string(),
        manifest_present,
        manifest_readable,
        update_check_disabled: std::env::var(UPDATE_DISABLE_ENV)
            .ok()
            .is_some_and(|v| !v.trim().is_empty()),
        github_token_configured: github_token_configured(),
        update_state_file: update_state_path,
        tools: tool_presence(),
        warnings,
    })
}

fn tool_presence() -> ToolPresence {
    ToolPresence {
        curl: command_exists("curl"),
        tar: command_exists("tar"),
        bash: command_exists("bash"),
        sha256sum: command_exists("sha256sum"),
        shasum: command_exists("shasum"),
        powershell: command_exists("powershell"),
    }
}

fn command_exists(name: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("where")
            .arg(name)
            .output()
            .ok()
            .is_some_and(|o| o.status.success())
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("which")
            .arg(name)
            .output()
            .ok()
            .is_some_and(|o| o.status.success())
    }
}

fn render(report: &DoctorReport, format: OutputFormat) -> Result<(), CommandFailure> {
    match format {
        OutputFormat::Text => {
            println!("mvs-manager doctor");
            println!("  version:           {}", report.version);
            println!("  current executable: {}", report.current_exe);
            if let Some(p) = &report.path_which {
                println!("  which mvs-manager:  {p}");
            } else {
                println!("  which mvs-manager:  (not found)");
            }
            println!("  PATH matches exe:   {}", report.path_on_path_matches_exe);
            println!(
                "  {} / {}: {}",
                MVS_REPO_ENV, MVS_UPDATE_REPO_ENV, report.repo_slug
            );
            println!(
                "  GitHub releases API: {}",
                report.github_latest_release_url
            );
            println!("  install.sh URL:      {}", report.install_sh_url);
            println!("  install.ps1 URL:   {}", report.install_ps1_url);
            println!("  manifest:            {}", report.manifest_path);
            println!(
                "  manifest readable:   {} (exists: {})",
                report.manifest_readable, report.manifest_present
            );
            println!("  {}: {}", UPDATE_DISABLE_ENV, report.update_check_disabled);
            let gh_tok = "GITHUB_TOKEN";
            println!(
                "  GitHub token set:    {} ({}/{gh_tok})",
                report.github_token_configured, UPDATE_GITHUB_TOKEN_ENV,
            );
            println!("  update state file:   {}", report.update_state_file);
            println!("  tools:");
            println!("    curl:        {}", report.tools.curl);
            println!("    tar:         {}", report.tools.tar);
            println!("    bash:        {}", report.tools.bash);
            println!("    sha256sum:   {}", report.tools.sha256sum);
            println!("    shasum:      {}", report.tools.shasum);
            println!("    powershell:  {}", report.tools.powershell);
            if !report.warnings.is_empty() {
                println!("\nWarnings:");
                for w in &report.warnings {
                    println!("  - {w}");
                }
            }
            println!("\nSelf-update safety:");
            println!("  {ALLOW_UNSAFE_SELF_UPDATE_ENV}: set to bypass Cargo/Nix/.cargo guards.");
            Ok(())
        }
        OutputFormat::Json => emit_json(report),
    }
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    command: &'static str,
    version: String,
    current_exe: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path_which: Option<String>,
    path_on_path_matches_exe: bool,
    repo_slug: String,
    github_latest_release_url: String,
    install_sh_url: String,
    install_ps1_url: String,
    manifest_path: String,
    manifest_present: bool,
    manifest_readable: bool,
    update_check_disabled: bool,
    github_token_configured: bool,
    update_state_file: String,
    tools: ToolPresence,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ToolPresence {
    curl: bool,
    tar: bool,
    bash: bool,
    sha256sum: bool,
    shasum: bool,
    powershell: bool,
}
