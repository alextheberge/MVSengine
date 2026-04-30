// SPDX-License-Identifier: AGPL-3.0-only
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::commands;
use crate::mvs::manifest::{
    GoExportFollowing, LuaExportFollowing, PythonExportFollowing, RubyExportFollowing,
    RustExportFollowing, TsExportFollowing,
};

pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_INIT_ERROR: i32 = 5;
pub const EXIT_GENERATE_ERROR: i32 = 10;
pub const EXIT_LINT_FAILED: i32 = 20;
pub const EXIT_LINT_ERROR: i32 = 21;
pub const EXIT_VALIDATE_INCOMPATIBLE: i32 = 30;
pub const EXIT_MANIFEST_ERROR: i32 = 40;
pub const EXIT_REPORT_ERROR: i32 = 50;
pub const EXIT_UPDATE_ERROR: i32 = 60;
pub const EXIT_OUTPUT_ERROR: i32 = 70;

#[derive(Debug, Parser)]
#[command(name = "mvs-manager", version, about = "MVS Engine manager CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init(InitArgs),
    Generate(GenerateArgs),
    Lint(LintArgs),
    Watch(WatchArgs),
    Validate(ValidateArgs),
    ValidateAll(ValidateAllArgs),
    CheckManifest(CheckManifestArgs),
    Constraint(ConstraintArgs),
    Report(ReportArgs),
    Schema(SchemaArgs),
    SelfUpdate(SelfUpdateArgs),
    Doctor(DoctorArgs),
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

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum TsExportFollowingArg {
    Off,
    RelativeOnly,
    WorkspaceOnly,
}

impl From<TsExportFollowingArg> for TsExportFollowing {
    fn from(value: TsExportFollowingArg) -> Self {
        match value {
            TsExportFollowingArg::Off => Self::Off,
            TsExportFollowingArg::RelativeOnly => Self::RelativeOnly,
            TsExportFollowingArg::WorkspaceOnly => Self::WorkspaceOnly,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum GoExportFollowingArg {
    Off,
    PackageOnly,
}

impl From<GoExportFollowingArg> for GoExportFollowing {
    fn from(value: GoExportFollowingArg) -> Self {
        match value {
            GoExportFollowingArg::Off => Self::Off,
            GoExportFollowingArg::PackageOnly => Self::PackageOnly,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum RustExportFollowingArg {
    Off,
    PublicModules,
}

impl From<RustExportFollowingArg> for RustExportFollowing {
    fn from(value: RustExportFollowingArg) -> Self {
        match value {
            RustExportFollowingArg::Off => Self::Off,
            RustExportFollowingArg::PublicModules => Self::PublicModules,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum RubyExportFollowingArg {
    Off,
    Heuristic,
}

impl From<RubyExportFollowingArg> for RubyExportFollowing {
    fn from(value: RubyExportFollowingArg) -> Self {
        match value {
            RubyExportFollowingArg::Off => Self::Off,
            RubyExportFollowingArg::Heuristic => Self::Heuristic,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum LuaExportFollowingArg {
    Off,
    ReturnedRootOnly,
    Heuristic,
}

impl From<LuaExportFollowingArg> for LuaExportFollowing {
    fn from(value: LuaExportFollowingArg) -> Self {
        match value {
            LuaExportFollowingArg::Off => Self::Off,
            LuaExportFollowingArg::ReturnedRootOnly => Self::ReturnedRootOnly,
            LuaExportFollowingArg::Heuristic => Self::Heuristic,
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct InitArgs {
    /// Root directory of the project to initialize.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,

    /// Path (relative to --root) where the manifest will be written.
    #[arg(long, default_value = "mvs.json")]
    pub manifest: PathBuf,

    /// Deployment context label (e.g. "cli", "lib", "plugin").
    #[arg(long)]
    pub context: Option<String>,

    /// Overwrite an existing manifest.
    #[arg(long, default_value_t = false)]
    pub force: bool,

    /// Print the generated manifest without writing it.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    /// Apply a named scan-policy preset: library, cli, plugin, plugin-host, sdk.
    #[arg(long, value_name = "PRESET")]
    pub preset: Option<String>,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
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

    #[arg(long = "ts-export-following", value_enum)]
    pub ts_export_following: Option<TsExportFollowingArg>,

    #[arg(long = "go-export-following", value_enum)]
    pub go_export_following: Option<GoExportFollowingArg>,

    #[arg(long = "rust-export-following", value_enum)]
    pub rust_export_following: Option<RustExportFollowingArg>,

    #[arg(long = "ruby-export-following", value_enum)]
    pub ruby_export_following: Option<RubyExportFollowingArg>,

    #[arg(long = "lua-export-following", value_enum)]
    pub lua_export_following: Option<LuaExportFollowingArg>,

    #[arg(long = "python-module-root")]
    pub python_module_roots: Vec<PathBuf>,

    #[arg(long = "rust-workspace-member")]
    pub rust_workspace_members: Vec<PathBuf>,

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

    /// Print per-failure remediation steps and list specific drifted symbols.
    #[arg(long, default_value_t = false)]
    pub explain: bool,

    /// Automatically run `generate` when drift is detected, then re-lint.
    #[arg(long, default_value_t = false)]
    pub remediate: bool,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct WatchArgs {
    #[arg(long, default_value = ".")]
    pub root: PathBuf,

    #[arg(long, default_value = "mvs.json")]
    pub manifest: PathBuf,

    #[arg(long)]
    pub ai_schema: Option<PathBuf>,

    #[arg(long, value_delimiter = ',')]
    pub available_model_capabilities: Vec<String>,

    /// Print per-failure remediation steps and list specific drifted symbols.
    #[arg(long, default_value_t = false)]
    pub explain: bool,

    /// Automatically run `generate` when drift is detected, then re-lint.
    #[arg(long, default_value_t = false)]
    pub remediate: bool,

    /// Run a single maintenance cycle and exit.
    #[arg(long, default_value_t = false, conflicts_with = "max_runs")]
    pub once: bool,

    /// Maximum number of watch cycles to run before exiting.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u64).range(1..))]
    pub max_runs: Option<u64>,

    /// Seconds to wait between watch cycles.
    #[arg(long, default_value_t = 30)]
    pub interval_secs: u64,

    /// Run lint every interval instead of only after detected workspace changes.
    #[arg(long, default_value_t = false)]
    pub run_every_interval: bool,

    /// Exit with a non-zero code if workspace fingerprinting fails instead of warning and running lint.
    #[arg(long, default_value_t = false)]
    pub strict_fingerprint: bool,
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

#[derive(Debug, Clone, Args)]
pub struct ValidateAllArgs {
    /// Directory to recursively search for mvs.json files.
    /// Mutually exclusive with explicit --manifest paths.
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,

    /// Explicit mvs.json paths to validate against each other.
    /// When provided, --dir is ignored.
    #[arg(long = "manifest", value_name = "PATH")]
    pub manifests: Vec<PathBuf>,

    /// Optional deployment context filter.
    #[arg(long)]
    pub context: Option<String>,

    /// Allow legacy shims to satisfy compatibility.
    #[arg(long, default_value_t = true)]
    pub allow_shims: bool,

    /// Only validate pairs that share the same ARCH value.
    #[arg(long, default_value_t = false)]
    pub same_arch_only: bool,

    /// Maximum directory depth when searching for mvs.json files.
    #[arg(long, default_value_t = 6)]
    pub max_depth: usize,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ReportArgs {
    #[arg(long)]
    pub base_manifest: PathBuf,

    #[arg(long)]
    pub target_manifest: PathBuf,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct CheckManifestArgs {
    /// Path to the manifest file to validate.
    #[arg(long, default_value = "mvs.json")]
    pub manifest: PathBuf,

    /// Root directory used to resolve relative `public_api_roots` paths.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ConstraintArgs {
    /// First manifest (acts as host in the suggested ranges).
    #[arg(long)]
    pub host_manifest: PathBuf,

    /// Second manifest (acts as extension in the suggested ranges).
    #[arg(long)]
    pub extension_manifest: PathBuf,

    /// Allow a looser range: extend N versions beyond the current PROT on each side.
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub lookahead: u64,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct SchemaArgs {
    /// Write the schema to a file instead of stdout.
    #[arg(long, value_name = "PATH")]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct SelfUpdateArgs {
    #[arg(long, default_value_t = false)]
    pub check: bool,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct DoctorArgs {
    /// Project root (used to resolve default manifest path for the report).
    #[arg(long, default_value = ".")]
    pub root: PathBuf,

    /// Manifest path relative to --root.
    #[arg(long, default_value = "mvs.json")]
    pub manifest: PathBuf,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

pub fn run() -> i32 {
    let cli = Cli::parse();

    match cli.command {
        Command::Init(args) => run_with_update_notification(commands::init::run(args)),
        Command::Generate(args) => run_with_update_notification(commands::generator::run(args)),
        Command::Lint(args) => run_with_update_notification(commands::linter::run(args)),
        Command::Watch(args) => commands::watch::run(args),
        Command::Validate(args) => run_with_update_notification(commands::reader::run(args)),
        Command::ValidateAll(args) => {
            run_with_update_notification(commands::validate_all::run(args))
        }
        Command::CheckManifest(args) => {
            run_with_update_notification(commands::check_manifest::run(args))
        }
        Command::Constraint(args) => run_with_update_notification(commands::constraint::run(args)),
        Command::Report(args) => run_with_update_notification(commands::report::run(args)),
        Command::Schema(args) => commands::schema::run(args),
        Command::SelfUpdate(args) => commands::self_update::run(args),
        Command::Doctor(args) => commands::doctor::run(args),
    }
}

fn run_with_update_notification(exit_code: i32) -> i32 {
    if exit_code == EXIT_SUCCESS {
        crate::update::maybe_notify_new_version();
    }
    exit_code
}
