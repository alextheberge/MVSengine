// SPDX-License-Identifier: AGPL-3.0-only
use serde::Serialize;

use crate::cli::{OutputFormat, SuggestDecoratorsArgs, EXIT_SUCCESS, EXIT_SUGGEST_ERROR};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::mvs::manifest::Manifest;
use crate::mvs::project_detect;
use crate::mvs::suggest::{self, SuggestionKind};

/// @mvs-feature("decorator_suggestions")
/// @mvs-protocol("cli_suggest_decorators_command")
pub fn run(args: SuggestDecoratorsArgs) -> i32 {
    match try_run(&args) {
        Ok(report) => match render_report(&report, args.format) {
            Ok(()) => EXIT_SUCCESS,
            Err(error) => emit_error(
                "suggest-decorators",
                args.format,
                error.exit_code,
                &error.message,
            ),
        },
        Err(error) => emit_error(
            "suggest-decorators",
            args.format,
            error.exit_code,
            &error.message,
        ),
    }
}

fn try_run(args: &SuggestDecoratorsArgs) -> Result<SuggestDecoratorsReport, CommandFailure> {
    let manifest_path = args.root.join(&args.manifest);
    let scan_policy = Manifest::load(&manifest_path)
        .map(|manifest| manifest.scan_policy)
        .unwrap_or_else(|_| {
            project_detect::detect_project(&args.root)
                .map(|detected| project_detect::build_scan_policy(&args.root, &detected, None))
                .unwrap_or_default()
        });

    let suggestions = suggest::build_suggestions(&args.root, &scan_policy)
        .map_err(|error| CommandFailure::new(EXIT_SUGGEST_ERROR, format!("{error:#}")))?;

    let mut protocol_suggestions = Vec::with_capacity(suggestions.protocols.len());
    for suggestion in &suggestions.protocols {
        let written = if args.write {
            Some(
                suggest::insert_decorator_comment(
                    &args.root,
                    &suggestion.file,
                    SuggestionKind::Protocol,
                    &suggestion.name,
                )
                .map_err(|error| {
                    CommandFailure::new(
                        EXIT_SUGGEST_ERROR,
                        format!(
                            "failed to write decorator into `{}`: {error:#}",
                            suggestion.file
                        ),
                    )
                })?,
            )
        } else {
            None
        };
        protocol_suggestions.push(ProtocolSuggestionPayload {
            name: suggestion.name.clone(),
            file: suggestion.file.clone(),
            item_count: suggestion.item_count,
            written,
        });
    }

    let mut feature_suggestions = Vec::with_capacity(suggestions.features.len());
    for suggestion in &suggestions.features {
        let written = if args.write {
            Some(
                suggest::insert_decorator_comment(
                    &args.root,
                    &suggestion.target_file,
                    SuggestionKind::Feature,
                    &suggestion.name,
                )
                .map_err(|error| {
                    CommandFailure::new(
                        EXIT_SUGGEST_ERROR,
                        format!(
                            "failed to write decorator into `{}`: {error:#}",
                            suggestion.target_file
                        ),
                    )
                })?,
            )
        } else {
            None
        };
        feature_suggestions.push(FeatureSuggestionPayload {
            name: suggestion.name.clone(),
            directory: suggestion.directory.clone(),
            target_file: suggestion.target_file.clone(),
            file_count: suggestion.file_count,
            written,
        });
    }

    let status = if protocol_suggestions.is_empty() && feature_suggestions.is_empty() {
        "no_suggestions"
    } else if args.write {
        "written"
    } else {
        "suggested"
    };

    Ok(SuggestDecoratorsReport {
        command: "suggest-decorators",
        status,
        root: args.root.display().to_string(),
        write: args.write,
        protocol_suggestions,
        feature_suggestions,
        commit_scopes_considered: suggestions.commit_scopes_considered,
    })
}

fn render_report(
    report: &SuggestDecoratorsReport,
    format: OutputFormat,
) -> std::result::Result<(), CommandFailure> {
    match format {
        OutputFormat::Text => {
            if report.status == "no_suggestions" {
                println!(
                    "suggest-decorators: no undecorated public API surface found under `{}`.",
                    report.root
                );
                return Ok(());
            }

            if !report.protocol_suggestions.is_empty() {
                println!("Protocol suggestions (one per undecorated file with public API):");
                for suggestion in &report.protocol_suggestions {
                    print_suggestion_line(
                        &suggestion.file,
                        &suggestion.name,
                        suggestion.item_count,
                        "item(s)",
                        suggestion.written,
                    );
                }
            }
            if !report.feature_suggestions.is_empty() {
                println!("Feature suggestions (one per undecorated directory):");
                for suggestion in &report.feature_suggestions {
                    print_suggestion_line(
                        &suggestion.target_file,
                        &suggestion.name,
                        suggestion.file_count,
                        "file(s) in this cluster",
                        suggestion.written,
                    );
                }
            }
            if !report.commit_scopes_considered.is_empty() {
                println!(
                    "Conventional Commit scopes seen in history: {}",
                    report.commit_scopes_considered.join(", ")
                );
            }

            let summary = match report.status {
                "written" => {
                    "decorators written; re-run without --write, or just re-run lint, to confirm."
                }
                _ => "run again with --write to insert these.",
            };
            println!("suggest-decorators: {summary}");

            Ok(())
        }
        OutputFormat::Json => emit_json(report).map_err(|error| {
            CommandFailure::new(
                EXIT_SUGGEST_ERROR,
                format!(
                    "failed to render suggest-decorators output: {}",
                    error.message
                ),
            )
        }),
    }
}

fn print_suggestion_line(
    target: &str,
    name: &str,
    count: usize,
    unit: &str,
    written: Option<bool>,
) {
    let marker = match written {
        Some(true) => "+",
        Some(false) => "x",
        None => "?",
    };
    println!("  {marker} {target}: @{name} ({count} {unit})");
    if written == Some(false) {
        println!("      (not written: unsupported file type for --write)");
    }
}

#[derive(Debug, Serialize)]
struct SuggestDecoratorsReport {
    command: &'static str,
    status: &'static str,
    root: String,
    write: bool,
    protocol_suggestions: Vec<ProtocolSuggestionPayload>,
    feature_suggestions: Vec<FeatureSuggestionPayload>,
    commit_scopes_considered: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ProtocolSuggestionPayload {
    name: String,
    file: String,
    item_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    written: Option<bool>,
}

#[derive(Debug, Serialize)]
struct FeatureSuggestionPayload {
    name: String,
    directory: String,
    target_file: String,
    file_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    written: Option<bool>,
}
