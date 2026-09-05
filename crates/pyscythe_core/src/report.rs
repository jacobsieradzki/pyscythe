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
}

/// Counts that summarise a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
