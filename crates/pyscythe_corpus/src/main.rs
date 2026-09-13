//! Runs pyscythe over pinned public projects and compares the reports with snapshots.
//!
//! `corpus/corpus.toml` names each project, its commit, and its environment;
//! `corpus/locks` pins the environment; `corpus/snapshots` holds the expected
//! output. `check` fails when the fresh output differs, `update` rewrites the
//! snapshots after a deliberate change, and `lock` re-resolves an environment.

use std::fs;
use std::io::Write;
use std::process::{Command, ExitCode};
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Parser, Subcommand};

mod analysis;
mod checkout;
mod environment;
mod manifest;
mod process;
mod snapshot;

use analysis::Analysis;
use manifest::{Manifest, ProjectName};
use snapshot::Comparison;

const DIFF_LINES_SHOWN: usize = 80;

#[derive(Debug, Parser)]
#[command(name = "pyscythe-corpus", version, about)]
struct Cli {
    /// The corpus manifest.
    #[arg(long, global = true, default_value = "corpus/corpus.toml")]
    manifest: Utf8PathBuf,

    /// Where clones and environments live. Defaults to `cache` beside the manifest.
    #[arg(long, global = true)]
    cache: Option<Utf8PathBuf>,

    /// The pyscythe binary to run. Defaults to `target/release/pyscythe`.
    #[arg(long, global = true)]
    binary: Option<Utf8PathBuf>,

    /// Only these projects; every project when omitted.
    #[arg(long, global = true, value_name = "NAME")]
    project: Vec<ProjectName>,

    /// Only these analyses; every analysis when omitted.
    #[arg(long, global = true, value_enum, value_name = "ANALYSIS")]
    only: Vec<Analysis>,

    #[command(subcommand)]
    mode: Mode,
}

#[derive(Debug, Clone, Copy, Subcommand)]
enum Mode {
    /// Run every analysis and fail if any output differs from its snapshot.
    Check,
    /// Run every analysis and rewrite the snapshots.
    Update,
    /// Re-resolve the projects' environments into the lock files.
    Lock,
}

/// Where the corpus keeps its files, all derived from the manifest's location.
#[derive(Debug)]
struct Layout {
    cache: Utf8PathBuf,
    locks: Utf8PathBuf,
    snapshots: Utf8PathBuf,
    binary: Utf8PathBuf,
}

impl Layout {
    fn from_cli(cli: &Cli) -> Result<Self> {
        // Every path is made absolute because uv and pyscythe run in the clones.
        let corpus = canonical(cli.manifest.parent().unwrap_or_else(|| Utf8Path::new(".")))?;
        let binary = cli.binary.clone().unwrap_or_else(|| {
            corpus
                .join("..")
                .join("target")
                .join("release")
                .join("pyscythe")
        });
        let cache = cli.cache.clone().unwrap_or_else(|| corpus.join("cache"));
        fs::create_dir_all(&cache).with_context(|| format!("creating {cache}"))?;
        Ok(Self {
            cache: canonical(&cache)?,
            locks: corpus.join("locks"),
            snapshots: corpus.join("snapshots"),
            binary: canonical(&binary).unwrap_or(binary),
        })
    }
}

fn canonical(path: &Utf8Path) -> Result<Utf8PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("resolving {path}"))?;
    Utf8PathBuf::from_path_buf(canonical)
        .map_err(|path| anyhow::anyhow!("{} is not UTF-8", path.display()))
}

/// The result of one analysis over one project.
#[derive(Debug)]
enum Outcome {
    Matched,
    Updated,
    Missing,
    Changed {
        added: usize,
        removed: usize,
        diff: String,
    },
    Failed(String),
}

impl Outcome {
    const fn is_success(&self) -> bool {
        matches!(self, Self::Matched | Self::Updated)
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let stdout = std::io::stdout();
    match run(&cli, &mut stdout.lock()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "error: {error:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: &Cli, out: &mut dyn Write) -> Result<bool> {
    let manifest = Manifest::load(&cli.manifest)?;
    let layout = Layout::from_cli(cli)?;
    for name in &cli.project {
        if !manifest.projects.contains_key(name) {
            bail!("no project `{name}` in {}", cli.manifest);
        }
    }
    let analyses: Vec<Analysis> = if cli.only.is_empty() {
        Analysis::ALL.to_vec()
    } else {
        cli.only.clone()
    };
    let selected = manifest
        .projects
        .iter()
        .filter(|(name, _)| cli.project.is_empty() || cli.project.contains(name));

    let mut all_succeeded = true;
    for (name, project) in selected {
        let clone = checkout::ensure(&layout.cache, name, project, out)?;
        let lock_file = environment::lock_file(&layout.locks, name);
        if matches!(cli.mode, Mode::Lock) {
            let scratch = layout.cache.join(format!("{name}.lock"));
            fs::create_dir_all(&scratch).with_context(|| format!("creating {scratch}"))?;
            environment::lock(&clone, project, &lock_file, &scratch)?;
            writeln!(out, "{name}: wrote {lock_file}")?;
            continue;
        }
        environment::ensure(name, &clone, project, &lock_file, out)?;
        let root = canonical(&clone.join(&project.root))?;
        for analysis in &analyses {
            let started = Instant::now();
            let outcome = analyse(&layout, cli.mode, name, &root, *analysis)?;
            let seconds = started.elapsed().as_secs_f64();
            all_succeeded &= outcome.is_success();
            report(out, name, *analysis, seconds, &outcome)?;
        }
    }
    if !all_succeeded {
        writeln!(
            out,
            "\nSnapshots differ. Review the diffs, then run `pyscythe-corpus update` to accept them."
        )?;
    }
    Ok(all_succeeded)
}

fn analyse(
    layout: &Layout,
    mode: Mode,
    name: &ProjectName,
    root: &Utf8Path,
    analysis: Analysis,
) -> Result<Outcome> {
    let fresh = match fresh_snapshot(layout, root, analysis)? {
        Ok(text) => text,
        Err(failure) => return Ok(Outcome::Failed(failure)),
    };
    let directory = layout.snapshots.join(name.as_str());
    let file = directory.join(analysis.snapshot_file());
    match mode {
        Mode::Update => {
            fs::create_dir_all(&directory).with_context(|| format!("creating {directory}"))?;
            fs::write(&file, fresh).with_context(|| format!("writing {file}"))?;
            Ok(Outcome::Updated)
        }
        Mode::Check => {
            let Ok(expected) = fs::read_to_string(&file) else {
                return Ok(Outcome::Missing);
            };
            Ok(match snapshot::compare(&expected, &fresh) {
                Comparison::Same => Outcome::Matched,
                Comparison::Different {
                    added,
                    removed,
                    diff,
                } => {
                    let diff_file = layout.cache.join(format!("{name}.{analysis}.diff"));
                    fs::write(&diff_file, &diff).with_context(|| format!("writing {diff_file}"))?;
                    Outcome::Changed {
                        added,
                        removed,
                        diff,
                    }
                }
            })
        }
        Mode::Lock => bail!("lock mode does not analyse"),
    }
}

/// Runs pyscythe and renders its report; the inner error is a failed run, reported not raised.
fn fresh_snapshot(
    layout: &Layout,
    root: &Utf8Path,
    analysis: Analysis,
) -> Result<Result<String, String>> {
    if !layout.binary.is_file() {
        bail!(
            "no pyscythe binary at {}: build it with `cargo build --release -p pyscythe`",
            layout.binary
        );
    }
    let output = Command::new(&layout.binary)
        .arg(analysis.subcommand())
        .args(["--format", "json"])
        .arg(root)
        .current_dir(root)
        // Only the `.venv` in the root may be visible to ty.
        .env_remove("VIRTUAL_ENV")
        .env_remove("CONDA_PREFIX")
        .output()
        .with_context(|| format!("running {}", layout.binary))?;
    let findings_or_none = matches!(output.status.code(), Some(0 | 1));
    if !findings_or_none {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Ok(Err(format!(
            "pyscythe {analysis} exited with {}: {}",
            output.status,
            stderr.trim()
        )));
    }
    let json = String::from_utf8_lossy(&output.stdout);
    Ok(snapshot::render(&json, root).map_err(|error| format!("{error:#}")))
}

fn report(
    out: &mut dyn Write,
    name: &ProjectName,
    analysis: Analysis,
    seconds: f64,
    outcome: &Outcome,
) -> Result<()> {
    let status = match outcome {
        Outcome::Matched => "ok".to_owned(),
        Outcome::Updated => "updated".to_owned(),
        Outcome::Missing => "MISSING snapshot: run `pyscythe-corpus update`".to_owned(),
        Outcome::Changed { added, removed, .. } => format!("CHANGED +{added} -{removed}"),
        Outcome::Failed(reason) => format!("FAILED {reason}"),
    };
    writeln!(out, "{name:<28} {analysis:<10} {seconds:>6.1}s  {status}")?;
    if let Outcome::Changed { diff, .. } = outcome {
        let mut lines = diff.lines();
        for line in lines.by_ref().take(DIFF_LINES_SHOWN) {
            writeln!(out, "    {line}")?;
        }
        let hidden = lines.count();
        if hidden > 0 {
            writeln!(
                out,
                "    ... {hidden} more lines in the cache as {name}.{analysis}.diff"
            )?;
        }
    }
    Ok(())
}
