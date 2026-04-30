// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Context;

use crate::cli::{InitArgs, OutputFormat, EXIT_GENERATE_ERROR, EXIT_INIT_ERROR, EXIT_SUCCESS};
use crate::commands::output::{emit_error, emit_json, CommandFailure};
use crate::mvs::manifest::{
    GoExportFollowing, Manifest, PythonExportFollowing, RustExportFollowing, ScanPolicy,
    TsExportFollowing,
};

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

// ── Project detection ────────────────────────────────────────────────────────

#[derive(Debug)]
struct ProjectDetection {
    languages: BTreeSet<&'static str>,
    markers: Vec<String>,
}

fn detect_project(root: &Path) -> anyhow::Result<ProjectDetection> {
    let mut languages: BTreeSet<&'static str> = BTreeSet::new();
    let mut markers: Vec<String> = Vec::new();

    // Check for project-level marker files first (these give strong signals)
    let marker_map: &[(&str, &str)] = &[
        ("Cargo.toml", "rust"),
        ("go.mod", "go"),
        ("pyproject.toml", "python"),
        ("setup.py", "python"),
        ("setup.cfg", "python"),
        ("requirements.txt", "python"),
        ("pubspec.yaml", "dart"),
        ("pom.xml", "java"),
        ("build.gradle", "kotlin"),
        ("build.gradle.kts", "kotlin"),
        ("*.csproj", "csharp"),
        ("*.sln", "csharp"),
        ("Gemfile", "ruby"),
        ("Package.swift", "swift"),
        ("composer.json", "php"),
    ];

    for (marker, lang) in marker_map {
        if marker.contains('*') {
            // glob-style check
            let prefix = marker.trim_start_matches('*');
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.ends_with(prefix) {
                        languages.insert(lang);
                        markers.push(name_str.to_string());
                        break;
                    }
                }
            }
        } else if root.join(marker).exists() {
            languages.insert(lang);
            markers.push((*marker).to_string());
        }
    }

    if root.join("package.json").exists() {
        markers.push("package.json".to_string());
        if root.join("tsconfig.json").exists() || root.join("jsconfig.json").exists() {
            languages.insert("typescript");
        } else {
            languages.insert("javascript");
        }
    }

    // If no markers found, fall back to extension counting
    if languages.is_empty() {
        let ext_map: &[(&str, &str)] = &[
            ("rs", "rust"),
            ("ts", "typescript"),
            ("tsx", "typescript"),
            ("js", "typescript"),
            ("go", "go"),
            ("py", "python"),
            ("java", "java"),
            ("kt", "kotlin"),
            ("cs", "csharp"),
            ("php", "php"),
            ("rb", "ruby"),
            ("swift", "swift"),
            ("lua", "lua"),
            ("luau", "luau"),
            ("liquid", "liquid"),
            ("dart", "dart"),
        ];
        scan_extensions(root, ext_map, &mut languages, 0)?;
    }

    Ok(ProjectDetection { languages, markers })
}

fn scan_extensions(
    dir: &Path,
    ext_map: &[(&'static str, &'static str)],
    languages: &mut BTreeSet<&'static str>,
    depth: usize,
) -> anyhow::Result<()> {
    if depth > 4 {
        return Ok(());
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // skip hidden dirs and common non-source dirs
        if name_str.starts_with('.') {
            continue;
        }
        if matches!(
            name_str.as_ref(),
            "target" | "node_modules" | "vendor" | "dist" | ".git" | "build" | "__pycache__"
        ) {
            continue;
        }
        if path.is_dir() {
            scan_extensions(&path, ext_map, languages, depth + 1)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            for (map_ext, lang) in ext_map {
                if ext == *map_ext {
                    languages.insert(lang);
                }
            }
        }
    }
    Ok(())
}

// ── Scan policy inference ────────────────────────────────────────────────────

fn build_scan_policy(root: &Path, detected: &ProjectDetection, preset: Option<&str>) -> ScanPolicy {
    let mut policy = ScanPolicy::default();

    // Common excludes regardless of language
    let default_excludes = ["target", "node_modules", "vendor", "dist", ".git", "build"];
    for excl in &default_excludes {
        if root.join(excl).exists() {
            policy.exclude_paths.push(excl.to_string());
        }
    }

    // Language-specific settings
    for lang in &detected.languages {
        match *lang {
            "rust" => {
                policy.rust_export_following = RustExportFollowing::PublicModules;
                // Try to detect the lib entry point
                for candidate in &["src/lib.rs", "src/main.rs"] {
                    if root.join(candidate).exists() {
                        policy.public_api_roots.push(candidate.to_string());
                        break;
                    }
                }
            }
            "typescript" | "javascript" => {
                policy.ts_export_following = TsExportFollowing::WorkspaceOnly;
                // Try to detect the main entry
                for candidate in &["src/index.ts", "index.ts", "src/index.js", "index.js"] {
                    if root.join(candidate).exists() {
                        policy.public_api_roots.push(candidate.to_string());
                        break;
                    }
                }
            }
            "go" => {
                policy.go_export_following = GoExportFollowing::PackageOnly;
            }
            "python" => {
                policy.python_export_following = PythonExportFollowing::Heuristic;
                // Try to detect python module root
                if let Ok(entries) = std::fs::read_dir(root) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() && path.join("__init__.py").exists() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            if !name.starts_with('.') {
                                policy.python_module_roots.push(name);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Apply preset overrides
    if let Some(preset_name) = preset {
        apply_preset(&mut policy, preset_name);
    }

    policy
}

fn apply_preset(policy: &mut ScanPolicy, preset: &str) {
    match preset {
        "library" => {
            policy
                .public_api_excludes
                .push("**/testdata/**".to_string());
        }
        "cli" => {
            // CLI tools: the public surface is just the CLI interface, not internal modules
            policy.public_api_excludes.push("*:fn main".to_string());
        }
        "plugin" => {
            // Plugins: strict protocol surface, exclude internals
            policy
                .public_api_excludes
                .push("**/internal/**".to_string());
            policy.public_api_excludes.push("**/tests/**".to_string());
        }
        "plugin-host" => {
            policy
                .public_api_excludes
                .push("**/internal/**".to_string());
            policy.public_api_excludes.push("**/tests/**".to_string());
            policy
                .public_api_excludes
                .push("**/fixtures/**".to_string());
        }
        "sdk" => {
            // SDKs: strict roots, explicit include lists encouraged
            policy.public_api_excludes.push("**/tests/**".to_string());
            policy
                .public_api_excludes
                .push("**/examples/**".to_string());
        }
        _ => {}
    }
}

fn infer_context(detected: &ProjectDetection) -> &'static str {
    // Use the first (alphabetically) detected language as context hint
    if let Some(lang) = detected.languages.iter().next() {
        match *lang {
            "typescript" | "javascript" | "luau" => "lib",
            "rust" => {
                // Can't easily distinguish lib vs cli without reading Cargo.toml here
                "lib"
            }
            _ => "lib",
        }
    } else {
        "lib"
    }
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
