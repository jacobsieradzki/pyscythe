//! The `pyscythe` command-line entry point.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use clap::{Parser, Subcommand, ValueEnum};
use pyscythe_core::baseline::Baseline;
use pyscythe_core::dupes::DupesOptions;
use pyscythe_core::finding::Confidence;
use pyscythe_core::keep::Policy;
use pyscythe_core::report::Report;
use pyscythe_core::tokens::CloneMode;
use pyscythe_pyproject::ProjectSettings;
use pyscythe_ty::{IndexOptions, TyIndex};

mod git;
mod render;
mod sarif;

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
    /// Report complexity hotspots and an overall health score.
    Health(AnalysisArgs),
    /// Report duplicated code.
    Dupes(DupesArgs),
    /// Report imports that cross the architecture boundaries in `[tool.pyscythe.boundaries]`.
    Boundaries(AnalysisArgs),
    /// Delete dead definitions and files. Shows a diff with --dry-run.
    Fix(FixArgs),
}

#[derive(Debug, clap::Args)]
struct FixArgs {
    /// Project root, or any path inside it. Defaults to the current directory.
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Print the changes as a unified diff instead of writing them.
    #[arg(long)]
    dry_run: bool,

    /// Only act on findings at this confidence or better.
    #[arg(long, value_enum, default_value_t = MinConfidence::Medium)]
    min_confidence: MinConfidence,

    /// Disable framework plugins.
    #[arg(long)]
    no_plugins: bool,

    /// Extra project-relative globs to leave alone.
    #[arg(long, value_name = "GLOB")]
    exclude: Vec<String>,

    /// Leave findings recorded in this baseline alone.
    #[arg(long, value_name = "FILE")]
    baseline: Option<PathBuf>,
}

impl FixArgs {
    fn as_analysis_args(&self) -> AnalysisArgs {
        AnalysisArgs {
            path: self.path.clone(),
            format: Format::Human,
            no_plugins: self.no_plugins,
            show_kept: false,
            exclude: self.exclude.clone(),
            timings: false,
            baseline: self.baseline.clone(),
            write_baseline: None,
            min_confidence: self.min_confidence,
            since: None,
        }
    }
}

#[derive(Debug, clap::Args)]
struct DupesArgs {
    #[command(flatten)]
    common: AnalysisArgs,

    /// How tokens are compared: exact, identifiers interchangeable, or identifiers and literals interchangeable.
    #[arg(long, value_enum, default_value_t = CloneModeArg::Mild)]
    mode: CloneModeArg,

    /// Shortest run of tokens that counts as a clone.
    #[arg(long, default_value_t = 50)]
    min_tokens: usize,

    /// Shortest run of lines that counts as a clone.
    #[arg(long, default_value_t = 5)]
    min_lines: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CloneModeArg {
    /// Tokens must match exactly.
    Strict,
    /// Renamed identifiers still match.
    Mild,
    /// Identifiers and literals are interchangeable.
    Weak,
}

impl From<CloneModeArg> for CloneMode {
    fn from(value: CloneModeArg) -> Self {
        match value {
            CloneModeArg::Strict => Self::Strict,
            CloneModeArg::Mild => Self::Mild,
            CloneModeArg::Weak => Self::Weak,
        }
    }
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

    /// Drop findings already recorded in this baseline file.
    #[arg(long, value_name = "FILE")]
    baseline: Option<PathBuf>,

    /// Record every current finding to this baseline file and exit 0.
    #[arg(long, value_name = "FILE")]
    write_baseline: Option<PathBuf>,

    /// Only report findings at this confidence or better.
    #[arg(long, value_enum, default_value_t = MinConfidence::Low)]
    min_confidence: MinConfidence,

    /// Only report findings in files changed since this git ref (plus untracked files).
    #[arg(long, value_name = "REF")]
    since: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum MinConfidence {
    /// Everything.
    Low,
    /// Medium and high.
    Medium,
    /// High only.
    High,
}

impl From<MinConfidence> for Confidence {
    fn from(value: MinConfidence) -> Self {
        match value {
            MinConfidence::Low => Self::Low,
            MinConfidence::Medium => Self::Medium,
            MinConfidence::High => Self::High,
        }
    }
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
    /// SARIF 2.1.0, for GitHub code scanning and editors.
    Sarif,
    /// GitHub Actions workflow annotations.
    Github,
    /// A Markdown table, for pull request comments.
    Markdown,
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
        Command::DeadCode(args) => run_analysis(&args, out, |i, a, s| Ok(dead_code(i, a, s))),
        Command::Cycles(args) => run_analysis(&args, out, |i, _, _| Ok(cycles(i))),
        Command::Health(args) => run_analysis(&args, out, |i, _, _| Ok(health(i))),
        Command::Boundaries(args) => run_analysis(&args, out, boundaries),
        Command::Fix(args) => run_fix(&args, out),
        Command::Dupes(args) => {
            let options = DupesOptions {
                mode: args.mode.into(),
                min_tokens: args.min_tokens,
                min_lines: args.min_lines,
            };
            run_analysis(&args.common, out, move |index, _, _| {
                Ok(pyscythe_core::dupes::analyze(index, &options))
            })
        }
    }
}

fn run_analysis(
    args: &AnalysisArgs,
    out: &mut impl std::io::Write,
    analysis: impl FnOnce(&TyIndex, &AnalysisArgs, &ProjectSettings) -> anyhow::Result<Report>,
) -> anyhow::Result<Outcome> {
    let mut timings = Timings::start();
    let (index, settings) = open_project(args, &mut timings)?;
    index.prepare();
    timings.mark("reference index");
    let mut report = analysis(&index, args, &settings)?;
    timings.mark("analysis");

    let root = index.root().to_path_buf();
    let wrote_baseline = write_baseline(args, &report, &root)?;
    apply_baseline(args, &mut report, &root)?;
    scope_to_changes(args, &mut report, &root)?;
    let threshold: Confidence = args.min_confidence.into();
    report
        .findings
        .retain(|finding| finding.confidence <= threshold);
    report.summary.findings = report.findings.len();

    emit(&report, args, &root, out)?;
    timings.mark("output");
    if args.timings {
        timings.report(&mut std::io::stderr().lock())?;
    }
    // Dropping the salsa database tears down every cached query one by one,
    // which costs more than the analysis did. The process is exiting anyway.
    std::mem::forget(index);
    Ok(if wrote_baseline || report.is_clean() {
        Outcome::Clean
    } else {
        Outcome::Findings
    })
}

/// Writes the current findings as a baseline when asked; returns whether it did.
fn write_baseline(args: &AnalysisArgs, report: &Report, root: &Utf8Path) -> anyhow::Result<bool> {
    let Some(path) = &args.write_baseline else {
        return Ok(false);
    };
    let baseline = Baseline::from_report(report, root);
    let text = serde_json::to_string_pretty(&baseline)?;
    std::fs::write(path, format!("{text}\n"))
        .map_err(|error| anyhow::anyhow!("cannot write baseline {}: {error}", path.display()))?;
    Ok(true)
}

/// With `--since`, keeps only findings that touch a changed file.
fn scope_to_changes(
    args: &AnalysisArgs,
    report: &mut Report,
    root: &Utf8Path,
) -> anyhow::Result<()> {
    let Some(reference) = &args.since else {
        return Ok(());
    };
    let changed = git::changed_files(root, reference)?;
    let touches_change = |path: &Utf8Path| changed.contains(&git::canonical(path));
    report.findings.retain(|finding| {
        touches_change(&finding.path)
            || match &finding.detail {
                pyscythe_core::finding::Detail::Cycle { chain } => {
                    chain.iter().any(|link| touches_change(&link.path))
                }
                _ => false,
            }
    });
    report.summary.findings = report.findings.len();
    report.summary.changed_files = Some(changed.len());
    Ok(())
}

fn apply_baseline(args: &AnalysisArgs, report: &mut Report, root: &Utf8Path) -> anyhow::Result<()> {
    let Some(path) = &args.baseline else {
        return Ok(());
    };
    let text = std::fs::read_to_string(path)
        .map_err(|error| anyhow::anyhow!("cannot read baseline {}: {error}", path.display()))?;
    let baseline: Baseline = serde_json::from_str(&text)
        .map_err(|error| anyhow::anyhow!("cannot parse baseline {}: {error}", path.display()))?;
    baseline.apply(report, root);
    Ok(())
}

/// Plans deletions for the dead-code findings and either shows or applies them.
fn run_fix(args: &FixArgs, out: &mut impl std::io::Write) -> anyhow::Result<Outcome> {
    let analysis_args = args.as_analysis_args();
    let mut timings = Timings::start();
    let (index, settings) = open_project(&analysis_args, &mut timings)?;
    index.prepare();
    let mut report = dead_code(&index, &analysis_args, &settings);
    let root = index.root().to_path_buf();
    apply_baseline(&analysis_args, &mut report, &root)?;
    let threshold: Confidence = args.min_confidence.into();
    report
        .findings
        .retain(|finding| finding.confidence <= threshold);

    let plan = pyscythe_core::fix::plan(&index, &report);
    std::mem::forget(index);

    for edit in &plan.edits {
        let diff = similar::TextDiff::from_lines(&edit.before, &edit.after);
        let relative = edit.path.strip_prefix(&root).unwrap_or(&edit.path);
        write!(
            out,
            "{}",
            diff.unified_diff()
                .context_radius(3)
                .header(&format!("a/{relative}"), &format!("b/{relative}"))
        )?;
    }
    for path in &plan.deletions {
        let relative = path.strip_prefix(&root).unwrap_or(path);
        writeln!(out, "delete {relative}")?;
    }
    for skipped in &plan.skipped {
        let relative = skipped
            .finding
            .path
            .strip_prefix(&root)
            .unwrap_or(&skipped.finding.path);
        writeln!(
            out,
            "skip {relative}: {} ({})",
            skipped.finding.message, skipped.reason
        )?;
    }

    if !args.dry_run {
        for edit in &plan.edits {
            std::fs::write(&edit.path, &edit.after)
                .map_err(|error| anyhow::anyhow!("cannot write {}: {error}", edit.path))?;
        }
        for path in &plan.deletions {
            std::fs::remove_file(path)
                .map_err(|error| anyhow::anyhow!("cannot delete {path}: {error}"))?;
        }
    }

    let verb = if args.dry_run {
        "Would remove"
    } else {
        "Removed"
    };
    writeln!(
        out,
        "\n{verb} {} definition(s) in {} file(s) and {} whole file(s); {} finding(s) skipped.",
        plan.removed_definitions(),
        plan.edits.len(),
        plan.deletions.len(),
        plan.skipped.len()
    )?;
    Ok(Outcome::Clean)
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

fn cycles(index: &TyIndex) -> Report {
    pyscythe_core::cycles::analyze(index)
}

fn health(index: &TyIndex) -> Report {
    pyscythe_core::health::analyze(index)
}

fn boundaries(
    index: &TyIndex,
    _args: &AnalysisArgs,
    settings: &ProjectSettings,
) -> anyhow::Result<Report> {
    let config = settings.config.boundaries.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "no boundaries configured: add `layers`, `rules`, or a `preset` under [tool.pyscythe.boundaries] in pyproject.toml"
        )
    })?;
    Ok(pyscythe_core::boundaries::analyze(index, config))
}

fn emit(
    report: &Report,
    args: &AnalysisArgs,
    root: &Utf8Path,
    out: &mut impl std::io::Write,
) -> anyhow::Result<()> {
    match args.format {
        Format::Human => render::human(report, args.show_kept, out)?,
        Format::Json => {
            serde_json::to_writer(&mut *out, report)?;
            writeln!(out)?;
        }
        Format::Sarif => {
            serde_json::to_writer_pretty(&mut *out, &sarif::document(report, root))?;
            writeln!(out)?;
        }
        Format::Github => render::github_annotations(report, root, out)?,
        Format::Markdown => render::markdown(report, root, out)?,
    }
    Ok(())
}
