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

    /// List the unreferenced symbols plugins kept, and why.
    #[arg(long)]
    show_kept: bool,

    /// Extra project-relative globs to leave out of reports, on top of `[tool.pyscythe] exclude`.
    #[arg(long, value_name = "GLOB")]
    exclude: Vec<String>,

    /// Print how long each phase took to stderr.
    #[arg(long)]
    timings: bool,
}

/// Wall-clock phases of a run, printed with `--timings`.
struct Timings {
    started: std::time::Instant,
    phases: Vec<(&'static str, std::time::Duration)>,
}

impl Timings {
    fn start() -> Self {
        Self {
            started: std::time::Instant::now(),
            phases: Vec::new(),
        }
    }

    fn mark(&mut self, phase: &'static str) {
        let now = std::time::Instant::now();
        self.phases.push((phase, now.duration_since(self.started)));
        self.started = now;
    }

    fn report(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        for (phase, duration) in &self.phases {
            writeln!(
                out,
                "{phase:>16}: {:>7.1} ms",
                duration.as_secs_f64() * 1000.0
            )?;
        }
        Ok(())
    }
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
    let mut timings = Timings::start();
    let (index, settings) = open_project(args, &mut timings)?;
    index.prepare();
    timings.mark("reference index");
    let report = analysis(&index, args, &settings);
    timings.mark("analysis");
    emit(&report, args, out)?;
    timings.mark("output");
    if args.timings {
        timings.report(&mut std::io::stderr().lock())?;
    }
    // Dropping the salsa database tears down every cached query one by one,
    // which costs more than the analysis did. The process is exiting anyway.
    std::mem::forget(index);
    Ok(if report.is_clean() {
        Outcome::Clean
    } else {
        Outcome::Findings
    })
}

/// Opens the project, reads its `pyproject.toml`, and applies the file selection it asks for.
fn open_project(
    args: &AnalysisArgs,
    timings: &mut Timings,
) -> anyhow::Result<(TyIndex, ProjectSettings)> {
    let mut index = TyIndex::open(&args.path)?;
    timings.mark("open project");
    let settings = pyscythe_pyproject::load(index.root())?;
    let options = IndexOptions {
        exclude: settings
            .config
            .exclude
            .extended(args.exclude.iter().map(String::as_str))?,
        notebooks: settings.config.notebooks,
    };
    index.select_files(&options)?;
    timings.mark("select files");
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

fn emit(report: &Report, args: &AnalysisArgs, out: &mut impl std::io::Write) -> anyhow::Result<()> {
    match args.format {
        Format::Human => render::human(report, args.show_kept, out)?,
        Format::Json => {
            serde_json::to_writer(&mut *out, report)?;
            writeln!(out)?;
        }
    }
    Ok(())
}
