//! Human-readable rendering of reports.

use std::io::Write;

use pyscythe_core::finding::Finding;
use pyscythe_core::report::{Report, ReportKind};

/// Writes one line per finding, optionally the kept symbols, then a summary.
pub(crate) fn human(report: &Report, show_kept: bool, out: &mut impl Write) -> std::io::Result<()> {
    for finding in &report.findings {
        writeln!(out, "{}", line_for(finding))?;
    }

    if show_kept && !report.kept.is_empty() {
        writeln!(out, "\nKept by plugins:")?;
        for kept in &report.kept {
            let location = kept.position.map_or_else(
                || kept.path.to_string(),
                |p| format!("{}:{}:{}", kept.path, p.line.get(), p.column.get()),
            );
            writeln!(
                out,
                "{location}  `{}` kept by {}: {}",
                kept.symbol.as_str(),
                kept.plugin.as_str(),
                kept.why
            )?;
        }
    }

    let summary = &report.summary;
    let details = match report.kind {
        ReportKind::DeadCode => {
            let mut parts = vec![format!("{} symbols checked", summary.symbols_checked)];
            if summary.symbols_kept > 0 {
                parts.push(format!("{} kept by plugins", summary.symbols_kept));
            }
            if summary.symbols_ignored > 0 {
                parts.push(format!("{} ignored by config", summary.symbols_ignored));
            }
            format!(" ({})", parts.join(", "))
        }
        ReportKind::Cycles => String::new(),
    };
    let subject = match report.kind {
        ReportKind::DeadCode => "dead code",
        ReportKind::Cycles => "import cycles",
    };
    if report.is_clean() {
        writeln!(
            out,
            "No {subject} found in {} files{details}.",
            summary.files_scanned
        )
    } else {
        writeln!(
            out,
            "\n{} finding(s) in {} files{details}.",
            summary.findings, summary.files_scanned
        )
    }
}

fn line_for(finding: &Finding) -> String {
    let location = finding.position.map_or_else(
        || finding.path.to_string(),
        |position| {
            format!(
                "{}:{}:{}",
                finding.path,
                position.line.get(),
                position.column.get()
            )
        },
    );
    format!("{location}  {}", finding.message)
}
