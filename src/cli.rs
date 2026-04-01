// SPDX-License-Identifier: AGPL-3.0-only
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::commands;
use crate::mvs::manifest::PythonExportFollowing;

pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_GENERATE_ERROR: i32 = 10;
pub const EXIT_LINT_FAILED: i32 = 20;
pub const EXIT_LINT_ERROR: i32 = 21;
pub const EXIT_VALIDATE_INCOMPATIBLE: i32 = 30;
pub const EXIT_MANIFEST_ERROR: i32 = 40;
pub const EXIT_OUTPUT_ERROR: i32 = 70;

#[derive(Debug, Parser)]
#[command(name = "mvs-manager", version, about = "MVS Engine manager CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Generate(GenerateArgs),
    Lint(LintArgs),
    Validate(ValidateArgs),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum PythonExportFollowingArg {
    Off,
    RootsOnly,
    Heuristic,
}

impl From<PythonExportFollowingArg> for PythonExportFollowing {
    fn from(value: PythonExportFollowingArg) -> Self {
        match value {
            PythonExportFollowingArg::Off => Self::Off,
            PythonExportFollowingArg::RootsOnly => Self::RootsOnly,
            PythonExportFollowingArg::Heuristic => Self::Heuristic,
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct GenerateArgs {
    #[arg(long, default_value = ".")]
    pub root: PathBuf,

    #[arg(long, default_value = "mvs.json")]
    pub manifest: PathBuf,

    #[arg(long)]
    pub context: Option<String>,

    #[arg(long)]
    pub ai_schema: Option<PathBuf>,

    #[arg(long, default_value_t = false)]
    pub arch_break: bool,

    #[arg(long)]
    pub arch_reason: Option<String>,

    #[arg(long, default_value_t = false, conflicts_with = "backwards_compatible")]
    pub lock_step: bool,

    #[arg(long, value_name = "N", conflicts_with = "lock_step")]
    pub backwards_compatible: Option<u64>,

    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    #[arg(long = "exclude-path")]
    pub exclude_paths: Vec<PathBuf>,

    #[arg(long = "public-api-root")]
    pub public_api_roots: Vec<PathBuf>,

    #[arg(long = "python-module-root")]
    pub python_module_roots: Vec<PathBuf>,

    #[arg(long = "python-export-following", value_enum)]
    pub python_export_following: Option<PythonExportFollowingArg>,

    #[arg(long = "public-api-include", value_name = "PATTERN")]
    pub public_api_includes: Vec<String>,

    #[arg(long = "public-api-exclude", value_name = "PATTERN")]
    pub public_api_excludes: Vec<String>,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct LintArgs {
    #[arg(long, default_value = ".")]
    pub root: PathBuf,

    #[arg(long, default_value = "mvs.json")]
    pub manifest: PathBuf,

    #[arg(long)]
    pub ai_schema: Option<PathBuf>,

    #[arg(long, value_delimiter = ',')]
    pub available_model_capabilities: Vec<String>,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ValidateArgs {
    #[arg(long)]
    pub host_manifest: PathBuf,

    #[arg(long)]
    pub extension_manifest: PathBuf,

    #[arg(long)]
    pub context: Option<String>,

    #[arg(long, default_value_t = true)]
    pub allow_shims: bool,

    #[arg(long, value_delimiter = ',')]
    pub host_model_capabilities: Vec<String>,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

pub fn run() -> i32 {
    let cli = Cli::parse();

    match cli.command {
        Command::Generate(args) => commands::generator::run(args),
        Command::Lint(args) => commands::linter::run(args),
        Command::Validate(args) => commands::reader::run(args),
    }
}
