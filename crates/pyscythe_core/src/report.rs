//! The aggregate result of running an analysis.

use serde::Serialize;

use crate::finding::Finding;

/// Which analysis produced a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReportKind {
    /// Unused symbols.
    DeadCode,
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
