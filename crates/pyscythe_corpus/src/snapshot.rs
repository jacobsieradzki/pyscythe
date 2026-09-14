//! Turning a JSON report into the stable text that is checked in and compared.
//!
//! Only what a rule change can move is kept: the summary counts and one sorted
//! line per finding. Paths are made relative to the analysed directory so the
//! snapshot is the same on every machine.

use std::fmt::Write as _;

use anyhow::{Context as _, Result};
use camino::Utf8Path;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Report {
    summary: Summary,
    findings: Vec<Finding>,
}

#[derive(Debug, Deserialize)]
struct Summary {
    files_scanned: u64,
    symbols_checked: u64,
    symbols_kept: u64,
    findings: u64,
    health: Option<Health>,
    duplication: Option<Duplication>,
}

#[derive(Debug, Deserialize)]
struct Health {
    score: u64,
    grade: String,
    functions: u64,
    max_cyclomatic: u64,
    max_cognitive: u64,
}

#[derive(Debug, Deserialize)]
struct Duplication {
    clones: u64,
    duplicated_lines: u64,
    total_lines: u64,
    percent_tenths: u64,
}

#[derive(Debug, Deserialize)]
struct Finding {
    rule: String,
    path: String,
    position: Option<Position>,
    confidence: String,
    message: String,
    symbol: Option<String>,
    distribution: Option<String>,
    function: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct Position {
    line: u64,
    column: u64,
}

/// One finding reduced to the fields a snapshot keeps, in sort order.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Line {
    path: String,
    row: u64,
    column: u64,
    rule: String,
    name: String,
    confidence: String,
    message: String,
}

impl Line {
    fn from_finding(finding: Finding, root: &str) -> Self {
        let (row, column) = finding
            .position
            .map_or((0, 0), |position| (position.line, position.column));
        let name = finding
            .symbol
            .or(finding.distribution)
            .or(finding.function)
            .unwrap_or_else(|| "-".to_owned());
        Self {
            path: relative(&finding.path, root),
            row,
            column,
            rule: finding.rule,
            name,
            confidence: finding.confidence,
            message: steady(&finding.message.replace(root, "")),
        }
    }

    fn render(&self, out: &mut String) {
        let location = if self.row == 0 {
            self.path.clone()
        } else {
            format!("{}:{}:{}", self.path, self.row, self.column)
        };
        out.push_str(&self.rule);
        out.push('\t');
        out.push_str(&self.confidence);
        out.push('\t');
        out.push_str(&location);
        out.push('\t');
        out.push_str(&self.name);
        out.push('\t');
        out.push_str(&self.message);
        out.push('\n');
    }
}

fn relative(path: &str, root: &str) -> String {
    path.strip_prefix(root).unwrap_or(path).to_owned()
}

/// Drops the one part of a message that is not a property of the code.
///
/// When several declared dependencies can bring in an undeclared one, which
/// is named depends on the environment the analysis ran in, so the snapshot
/// records that a carrier exists rather than which one it was.
fn steady(message: &str) -> String {
    message.find(" it arrives through `").map_or_else(
        || message.to_owned(),
        |at| {
            format!(
                "{} it arrives through a declared dependency",
                &message[..at]
            )
        },
    )
}

/// Renders a pyscythe JSON report as snapshot text, with paths relative to `root`.
pub(crate) fn render(json: &str, root: &Utf8Path) -> Result<String> {
    let report: Report = serde_json::from_str(json).context("parsing the JSON report")?;
    let mut prefix = root.to_string();
    if !prefix.ends_with('/') {
        prefix.push('/');
    }
    let mut lines = report
        .findings
        .into_iter()
        .map(|finding| Line::from_finding(finding, &prefix))
        .collect::<Vec<_>>();
    lines.sort();

    let summary = report.summary;
    let mut out = String::new();
    // Counts are fixed-width free text; write! into a String cannot fail.
    let _ = writeln!(out, "files_scanned={}", summary.files_scanned);
    let _ = writeln!(out, "symbols_checked={}", summary.symbols_checked);
    let _ = writeln!(out, "symbols_kept={}", summary.symbols_kept);
    let _ = writeln!(out, "findings={}", summary.findings);
    if let Some(health) = summary.health {
        let _ = writeln!(
            out,
            "health: score={} grade={} functions={} max_cyclomatic={} max_cognitive={}",
            health.score,
            health.grade,
            health.functions,
            health.max_cyclomatic,
            health.max_cognitive
        );
    }
    if let Some(duplication) = summary.duplication {
        let _ = writeln!(
            out,
            "duplication: clones={} duplicated_lines={} total_lines={} percent_tenths={}",
            duplication.clones,
            duplication.duplicated_lines,
            duplication.total_lines,
            duplication.percent_tenths
        );
    }
    out.push_str("--\n");
    for line in &lines {
        line.render(&mut out);
    }
    Ok(out)
}

/// How the fresh snapshot text relates to the checked-in one.
#[derive(Debug)]
pub(crate) enum Comparison {
    Same,
    Different {
        added: usize,
        removed: usize,
        diff: String,
    },
}

/// Compares snapshot texts line by line, keeping a unified diff for the report.
pub(crate) fn compare(expected: &str, actual: &str) -> Comparison {
    if expected == actual {
        return Comparison::Same;
    }
    let text_diff = similar::TextDiff::from_lines(expected, actual);
    let (mut added, mut removed) = (0, 0);
    for change in text_diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => added += 1,
            similar::ChangeTag::Delete => removed += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    let diff = text_diff
        .unified_diff()
        .context_radius(1)
        .header("snapshot", "fresh")
        .to_string();
    Comparison::Different {
        added,
        removed,
        diff,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = r#"{
      "schema_version": 1, "kind": "dead-code",
      "summary": {"files_scanned": 3, "symbols_checked": 10, "symbols_kept": 4, "symbols_ignored": 0, "suppressed": 0, "baselined": 0, "findings": 2},
      "findings": [
        {"rule": "unused-function", "path": "/work/app/b.py", "module": "app.b", "position": {"line": 9, "column": 5}, "confidence": "medium", "message": "function `later` is never used", "symbol": "later"},
        {"rule": "unused-dependency", "path": "/work/app/pyproject.toml", "module": null, "position": null, "confidence": "low", "message": "dependency `six` arrives through /work/app/x", "distribution": "six", "modules": []},
        {"rule": "unused-class", "path": "/work/app/a.py", "module": "app.a", "position": {"line": 2, "column": 7}, "confidence": "high", "message": "class `First` is never used", "symbol": "First"}
      ],
      "kept": []
    }"#;

    #[test]
    fn renders_sorted_relative_lines_under_the_summary() {
        let text = render(REPORT, Utf8Path::new("/work/app")).unwrap();
        assert_eq!(
            text,
            "files_scanned=3\nsymbols_checked=10\nsymbols_kept=4\nfindings=2\n--\n\
unused-class\thigh\ta.py:2:7\tFirst\tclass `First` is never used\n\
unused-function\tmedium\tb.py:9:5\tlater\tfunction `later` is never used\n\
unused-dependency\tlow\tpyproject.toml\tsix\tdependency `six` arrives through x\n"
        );
    }

    #[test]
    fn the_carrier_of_a_transitive_dependency_is_not_recorded() {
        assert_eq!(
            steady("`six` is not a declared dependency; it arrives through `rich`"),
            "`six` is not a declared dependency; it arrives through a declared dependency"
        );
        assert_eq!(steady("`six` is never used"), "`six` is never used");
    }

    #[test]
    fn counts_added_and_removed_lines() {
        match compare("a\nb\nc\n", "a\nc\nd\n") {
            Comparison::Different {
                added,
                removed,
                diff,
            } => {
                assert_eq!((added, removed), (1, 1));
                assert!(diff.contains("-b"), "{diff}");
                assert!(diff.contains("+d"), "{diff}");
            }
            Comparison::Same => panic!("texts differ"),
        }
        assert!(matches!(compare("x\n", "x\n"), Comparison::Same));
    }
}
