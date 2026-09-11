//! The aggregate result of running an analysis.

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::finding::Finding;
use crate::keep::PluginName;
use crate::source::{ModulePath, Position};
use crate::symbol::SymbolName;

/// An unreferenced symbol a plugin decided to keep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeptSymbol {
    /// Absolute path of the file.
    pub path: Utf8PathBuf,
    /// Module path of the file, when resolvable.
    pub module: Option<ModulePath>,
    /// The symbol kept.
    pub symbol: SymbolName,
    /// Where its name appears.
    pub position: Option<Position>,
    /// The plugin that kept it.
    pub plugin: PluginName,
    /// Why, in a few words.
    pub why: &'static str,
}

/// Which analysis produced a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReportKind {
    /// Unused symbols and files.
    DeadCode,
    /// Circular imports.
    Cycles,
    /// Complexity hotspots and an overall score.
    Health,
    /// Duplicated code.
    Dupes,
    /// Architecture boundary violations.
    Boundaries,
    /// Declared versus imported dependencies.
    Deps,
}

impl ReportKind {
    /// The kebab-case name used in JSON and comment markers.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::DeadCode => "dead-code",
            Self::Cycles => "cycles",
            Self::Health => "health",
            Self::Dupes => "dupes",
            Self::Boundaries => "boundaries",
            Self::Deps => "deps",
        }
    }
}

/// How much of the project is duplicated, from the `dupes` analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DuplicationSummary {
    /// Clone pairs found.
    pub clones: usize,
    /// Lines covered by some clone occurrence.
    pub duplicated_lines: u32,
    /// Lines of code considered.
    pub total_lines: u32,
    /// `duplicated_lines / total_lines`, in tenths of a percent.
    pub percent_tenths: u32,
}

/// A letter grade for a health score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Grade {
    /// 90 and above.
    A,
    /// 80 to 89.
    B,
    /// 70 to 79.
    C,
    /// 60 to 69.
    D,
    /// Below 60.
    F,
}

impl Grade {
    /// The grade for a 0 to 100 score.
    #[must_use]
    pub const fn for_score(score: u8) -> Self {
        match score {
            90..=u8::MAX => Self::A,
            80..=89 => Self::B,
            70..=79 => Self::C,
            60..=69 => Self::D,
            _ => Self::F,
        }
    }

    /// The letter as text.
    #[must_use]
    pub const fn letter(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
            Self::F => "F",
        }
    }
}

/// One file's share of the health picture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileHealth {
    /// Absolute path.
    pub path: Utf8PathBuf,
    /// Module path, when resolvable.
    pub module: Option<ModulePath>,
    /// 0 to 100 from the same penalty scheme as the project score.
    pub score: u8,
    /// Maintainability index on the 0 to 100 scale radon uses.
    pub maintainability: u8,
    /// Functions in the file.
    pub functions: usize,
    /// Functions over a hotspot threshold.
    pub hotspots: usize,
}

/// Overall complexity health, from the `health` analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthSummary {
    /// 0 to 100, where 100 means every function is under every threshold.
    pub score: u8,
    /// The letter grade for `score`.
    pub grade: Grade,
    /// Functions measured.
    pub functions: usize,
    /// Highest cyclomatic complexity seen.
    pub max_cyclomatic: u32,
    /// Highest cognitive complexity seen.
    pub max_cognitive: u32,
    /// The files with the lowest scores, worst first, at most ten.
    pub worst_files: Vec<FileHealth>,
}

/// Counts that summarise a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// First-party files the index knew about.
    pub files_scanned: usize,
    /// Symbols the analysis considered.
    pub symbols_checked: usize,
    /// Unreferenced symbols a framework plugin kept.
    pub symbols_kept: usize,
    /// Candidate symbols skipped by `ignore-names` configuration.
    pub symbols_ignored: usize,
    /// Findings silenced by `# pyscythe: ignore` comments.
    pub suppressed: usize,
    /// Findings already recorded in the baseline.
    pub baselined: usize,
    /// Findings produced.
    pub findings: usize,
    /// When `--since` scoped the run, how many changed files were considered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_files: Option<usize>,
    /// Present for the `health` analysis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<HealthSummary>,
    /// Present for the `dupes` analysis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplication: Option<DuplicationSummary>,
}

/// A complete, serialisable analysis result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Bumped when the JSON shape changes incompatibly.
    pub schema_version: u32,
    /// Which analysis ran.
    pub kind: ReportKind,
    /// Findings sorted by path then position.
    pub findings: Vec<Finding>,
    /// Unreferenced symbols plugins kept, sorted by path then position.
    pub kept: Vec<KeptSymbol>,
    /// Run counts.
    pub summary: Summary,
}

impl Report {
    /// The current JSON schema version.
    pub const SCHEMA_VERSION: u32 = 1;

    /// Whether the run produced no findings.
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}
