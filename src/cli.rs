use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

use crate::commands;

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

    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Args)]
pub struct LintArgs {
    #[arg(long, default_value = ".")]
    pub root: PathBuf,

    #[arg(long, default_value = "mvs.json")]
    pub manifest: PathBuf,

    #[arg(long)]
    pub ai_schema: Option<PathBuf>,
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
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Generate(args) => commands::generator::run(args),
        Command::Lint(args) => commands::linter::run(args),
        Command::Validate(args) => commands::reader::run(args),
    }
}
