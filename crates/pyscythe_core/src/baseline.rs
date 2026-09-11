//! A record of findings a team has decided to live with for now.
//!
//! Keys deliberately leave out line numbers so a baseline survives edits
//! elsewhere in the file.

use std::collections::BTreeSet;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::finding::{Detail, Finding, Rule};
use crate::report::Report;
use crate::symbol::SymbolName;

/// Identifies a finding independently of where in the file it sits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BaselineKey {
    /// The rule that fired.
    pub rule: Rule,
    /// Project-relative path.
    pub path: Utf8PathBuf,
    /// The symbol, for symbol findings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<SymbolName>,
    /// The owning class, for methods.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<SymbolName>,
    /// The line, only for findings that have no name of their own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

impl BaselineKey {
    /// The key for `finding`, with its path made relative to `root`.
    #[must_use]
    pub fn of(finding: &Finding, root: &Utf8Path) -> Self {
        let (symbol, owner, line) = match &finding.detail {
            Detail::Symbol { symbol, owner } => (Some(symbol.clone()), owner.clone(), None),
            Detail::Metrics { function, .. } => {
                (Some(SymbolName::new(function.clone())), None, None)
            }
            Detail::Comment | Detail::Duplicate { .. } => {
                (None, None, finding.position.map(|p| p.line.get()))
            }
            Detail::Import { to_module, .. } => {
                (Some(SymbolName::new(to_module.as_str())), None, None)
            }
            Detail::Cycle { .. } | Detail::File => (None, None, None),
        };
        Self {
            rule: finding.rule,
            path: finding
                .path
                .strip_prefix(root)
                .map_or_else(|_| finding.path.clone(), Utf8Path::to_path_buf),
            symbol,
            owner,
            line,
        }
    }
}

/// Findings accepted as pre-existing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    /// Bumped when the file shape changes incompatibly.
    pub schema_version: u32,
    /// The accepted findings.
    pub findings: BTreeSet<BaselineKey>,
}

impl Baseline {
    /// The current file schema version.
    pub const SCHEMA_VERSION: u32 = 1;

    /// A baseline recording every finding in `report`.
    #[must_use]
    pub fn from_report(report: &Report, root: &Utf8Path) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            findings: report
                .findings
                .iter()
                .map(|finding| BaselineKey::of(finding, root))
                .collect(),
        }
    }

    /// Removes findings recorded in this baseline from `report`, returning
    /// how many were removed and updating the summary.
    pub fn apply(&self, report: &mut Report, root: &Utf8Path) -> usize {
        let before = report.findings.len();
        report
            .findings
            .retain(|finding| !self.findings.contains(&BaselineKey::of(finding, root)));
        let removed = before - report.findings.len();
        report.summary.baselined += removed;
        report.summary.findings = report.findings.len();
        removed
    }
}

#[cfg(test)]
mod tests {
    use camino::{Utf8Path, Utf8PathBuf};

    use super::{Baseline, BaselineKey};
    use crate::finding::{Confidence, Detail, Finding, Rule};
    use crate::report::{Report, ReportKind, Summary};
    use crate::symbol::SymbolName;

    fn finding(path: &str, symbol: &str) -> Finding {
        Finding {
            rule: Rule::UnusedFunction,
            path: Utf8PathBuf::from(path),
            module: None,
            position: None,
            confidence: Confidence::Medium,
            message: String::new(),
            detail: Detail::Symbol {
                symbol: SymbolName::new(symbol),
                owner: None,
            },
        }
    }

    fn report(findings: Vec<Finding>) -> Report {
        Report {
            schema_version: Report::SCHEMA_VERSION,
            kind: ReportKind::DeadCode,
            summary: Summary {
                files_scanned: 1,
                symbols_checked: findings.len(),
                symbols_kept: 0,
                symbols_ignored: 0,
                suppressed: 0,
                baselined: 0,
                findings: findings.len(),
                changed_files: None,
                health: None,
                duplication: None,
            },
            findings,
            kept: Vec::new(),
        }
    }

    #[test]
    fn keys_are_relative_to_the_root_and_ignore_positions() {
        let key = BaselineKey::of(&finding("/proj/pkg/a.py", "f"), Utf8Path::new("/proj"));
        assert_eq!(key.path, Utf8PathBuf::from("pkg/a.py"));
        assert_eq!(key.symbol.as_ref().unwrap().as_str(), "f");
    }

    #[test]
    fn applying_a_baseline_removes_known_findings_only() {
        let root = Utf8Path::new("/proj");
        let old = report(vec![finding("/proj/pkg/a.py", "old")]);
        let baseline = Baseline::from_report(&old, root);

        let mut current = report(vec![
            finding("/proj/pkg/a.py", "old"),
            finding("/proj/pkg/a.py", "new"),
        ]);
        let removed = baseline.apply(&mut current, root);

        assert_eq!(removed, 1);
        assert_eq!(current.findings.len(), 1);
        assert_eq!(current.findings[0].symbol().unwrap().as_str(), "new");
        assert_eq!(current.summary.baselined, 1);
        assert_eq!(current.summary.findings, 1);
    }

    #[test]
    fn round_trips_through_json() {
        let root = Utf8Path::new("/proj");
        let baseline = Baseline::from_report(&report(vec![finding("/proj/pkg/a.py", "f")]), root);
        let text = serde_json::to_string(&baseline).unwrap();
        let parsed: Baseline = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, baseline);
    }
}
