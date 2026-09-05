//! The `pyscythe` command-line entry point.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use pyscythe_core::keep::Policy;
use pyscythe_core::report::Report;
use pyscythe_pyproject::ProjectSettings;
use pyscythe_ty::{IndexOptions, TyIndex};

mod render;

/// Codebase intelligence for Python.
#[derive(Debug, Parser)]
#[command(name = "pyscythe", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Report module-level definitions and files that nothing refers to.
    DeadCode(AnalysisArgs),
    /// Report groups of modules that import each other at load time.
    Cycles(AnalysisArgs),
}

#[derive(Debug, clap::Args)]
struct AnalysisArgs {
    /// Project root, or any path inside it. Defaults to the current directory.
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Human)]
    format: Format,

    /// Disable framework plugins, reporting every unreferenced symbol.
    #[arg(long)]
    no_plugins: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    /// A terminal report for people.
    Human,
    /// A compact JSON document for tools.
    Json,
}

/// Process exit statuses, mirroring the fallow convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Clean,
    Findings,
    Error,
}

impl From<Outcome> for ExitCode {
    fn from(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Clean => Self::from(0),
            Outcome::Findings => Self::from(1),
            Outcome::Error => Self::from(2),
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();

    match run(cli, &mut stdout.lock()) {
        Ok(outcome) => outcome.into(),
        Err(error) => {
            // Best effort: if stderr is closed there is nowhere left to report to.
            let _ = writeln!(stderr.lock(), "error: {error:#}");
            Outcome::Error.into()
        }
    }
}

fn run(cli: Cli, out: &mut impl std::io::Write) -> anyhow::Result<Outcome> {
    match cli.command {
        Command::DeadCode(args) => run_analysis(&args, out, dead_code),
        Command::Cycles(args) => run_analysis(&args, out, cycles),
    }
}

fn run_analysis(
    args: &AnalysisArgs,
    out: &mut impl std::io::Write,
    analysis: impl FnOnce(&TyIndex, &AnalysisArgs, &ProjectSettings) -> Report,
) -> anyhow::Result<Outcome> {
    let (index, settings) = open_project(&args.path)?;
    let report = analysis(&index, args, &settings);
    emit(&report, args.format, out)?;
    Ok(if report.is_clean() {
        Outcome::Clean
    } else {
        Outcome::Findings
    })
}

/// Reads settings from the project's `pyproject.toml`, then opens the index with them.
///
/// ty decides where the project root is, so the index is opened once to learn
/// it and again with the exclusions the settings ask for.
fn open_project(path: &std::path::Path) -> anyhow::Result<(TyIndex, ProjectSettings)> {
    let probe = TyIndex::open(path, &IndexOptions::default())?;
    let settings = pyscythe_pyproject::load(probe.root())?;
    let options = IndexOptions {
        exclude: settings.config.exclude.clone(),
        notebooks: settings.config.notebooks,
    };
    drop(probe);
    let index = TyIndex::open(path, &options)?;
    Ok((index, settings))
}

fn dead_code(index: &TyIndex, args: &AnalysisArgs, settings: &ProjectSettings) -> Report {
    let policy = if args.no_plugins {
        Policy::none()
    } else {
        Policy::builtin()
    };
    pyscythe_core::dead_code::analyze(index, &policy, &settings.manifest, &settings.config)
}

fn cycles(index: &TyIndex, _args: &AnalysisArgs, _settings: &ProjectSettings) -> Report {
    pyscythe_core::cycles::analyze(index)
}

fn emit(report: &Report, format: Format, out: &mut impl std::io::Write) -> anyhow::Result<()> {
    match format {
        Format::Human => render::human(report, out)?,
        Format::Json => {
            serde_json::to_writer(&mut *out, report)?;
            writeln!(out)?;
        }
    }
    Ok(())
}
