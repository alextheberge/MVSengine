// SPDX-License-Identifier: AGPL-3.0-only
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, UNIX_EPOCH},
};

use crate::cli::{LintArgs, OutputFormat, WatchArgs, EXIT_SUCCESS};
use crate::mvs::hashing::hash_items;
use crate::mvs::manifest::{Manifest, ScanPolicy};

/// @mvs-feature("manifest_watch")
/// @mvs-protocol("cli_watch_command")
pub fn run(args: WatchArgs) -> i32 {
    let max_cycles = if args.once { Some(1) } else { args.max_runs };
    let interval = Duration::from_secs(args.interval_secs);

    if !args.once {
        let cadence = if args.run_every_interval {
            "running maintenance every interval"
        } else {
            "running maintenance when the workspace changes"
        };
        println!(
            "Watching {} every {}s, {}. Press Ctrl-C to stop.",
            args.root.display(),
            args.interval_secs,
            cadence
        );
    }

    let mut cycle_count = 0_u64;
    let mut run_count = 0_u64;
    let mut skipped_count = 0_u64;
    let mut last_exit_code = EXIT_SUCCESS;
    let mut last_fingerprint = None;

    loop {
        cycle_count += 1;

        let should_run = if cycle_count == 1 || args.run_every_interval {
            true
        } else {
            match workspace_fingerprint(&args.root, &args.manifest, args.ai_schema.as_deref()) {
                Ok(current) => last_fingerprint.as_deref() != Some(current.as_str()),
                Err(error) => {
                    eprintln!(
                        "warning: failed to fingerprint workspace before cycle {cycle_count}: {error}"
                    );
                    true
                }
            }
        };

        if should_run {
            if !args.once {
                println!("\n[watch cycle {cycle_count}]");
            }
            last_exit_code = run_lint_cycle(&args);
            run_count += 1;

            match workspace_fingerprint(&args.root, &args.manifest, args.ai_schema.as_deref()) {
                Ok(fingerprint) => {
                    last_fingerprint = Some(fingerprint);
                }
                Err(error) => {
                    eprintln!(
                        "warning: failed to fingerprint workspace after cycle {cycle_count}: {error}"
                    );
                    last_fingerprint = None;
                }
            }

            if last_exit_code == EXIT_SUCCESS {
                crate::update::maybe_notify_new_version();
            }
        } else {
            skipped_count += 1;
        }

        if let Some(limit) = max_cycles {
            if cycle_count >= limit {
                println!(
                    "\nWatch summary: {cycle_count} cycle(s), {run_count} maintenance run(s), {skipped_count} skipped, last exit code {last_exit_code}."
                );
                return last_exit_code;
            }
        }

        thread::sleep(interval);
    }
}

fn run_lint_cycle(args: &WatchArgs) -> i32 {
    crate::commands::linter::run(LintArgs {
        root: args.root.clone(),
        manifest: args.manifest.clone(),
        ai_schema: args.ai_schema.clone(),
        available_model_capabilities: args.available_model_capabilities.clone(),
        explain: args.explain,
        remediate: args.remediate,
        format: OutputFormat::Text,
    })
}

fn workspace_fingerprint(
    root: &Path,
    manifest: &Path,
    ai_schema: Option<&Path>,
) -> Result<String, String> {
    let scan_policy = load_scan_policy(manifest);
    let mut entries = Vec::new();

    let walker = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| !should_skip_entry(root, entry, &scan_policy));

    for entry in walker {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry.file_type().is_file() {
            continue;
        }

        let relative = relative_path(root, entry.path());
        if let Some(marker) = file_marker(entry.path(), &relative)? {
            entries.push(marker);
        }
    }

    add_external_file_marker(&mut entries, root, manifest, "manifest")?;
    if let Some(ai_schema) = ai_schema {
        add_external_file_marker(&mut entries, root, ai_schema, "ai_schema")?;
    }

    entries.sort();
    Ok(hash_items(entries))
}

fn load_scan_policy(manifest: &Path) -> ScanPolicy {
    if manifest.exists() {
        Manifest::load(manifest)
            .map(|loaded| loaded.scan_policy)
            .unwrap_or_default()
    } else {
        ScanPolicy::default()
    }
}

fn should_skip_entry(root: &Path, entry: &walkdir::DirEntry, scan_policy: &ScanPolicy) -> bool {
    if entry.depth() == 0 {
        return false;
    }

    let relative = relative_path(root, entry.path());
    if scan_policy.matching_excluded_path(&relative).is_some() {
        return true;
    }

    entry.file_type().is_dir()
        && entry
            .file_name()
            .to_str()
            .and_then(default_ignored_directory_name)
            .is_some()
}

fn default_ignored_directory_name(name: &str) -> Option<&'static str> {
    match name {
        ".git" => Some(".git"),
        "node_modules" => Some("node_modules"),
        "dist" => Some("dist"),
        "build" => Some("build"),
        "target" => Some("target"),
        "vendor" => Some("vendor"),
        ".next" => Some(".next"),
        "tests" => Some("tests"),
        "examples" => Some("examples"),
        "benches" => Some("benches"),
        _ => None,
    }
}

fn add_external_file_marker(
    entries: &mut Vec<String>,
    root: &Path,
    path: &Path,
    kind: &str,
) -> Result<(), String> {
    if path.strip_prefix(root).is_ok() {
        return Ok(());
    }

    if let Some(marker) = file_marker(path, &format!("{kind}:{}", path.display()))? {
        entries.push(marker);
    }
    Ok(())
}

fn file_marker(path: &Path, label: &str) -> Result<Option<String>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to read metadata for {}: {error}",
                path.display()
            ))
        }
    };

    let modified = metadata
        .modified()
        .ok()
        .and_then(|timestamp| timestamp.duration_since(UNIX_EPOCH).ok())
        .map(|duration| format!("{}:{}", duration.as_secs(), duration.subsec_nanos()))
        .unwrap_or_else(|| "0:0".to_string());

    Ok(Some(format!("{label}|{}|{modified}", metadata.len())))
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
