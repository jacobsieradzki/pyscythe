//! Human-readable rendering of reports.

use std::io::Write;

use pyscythe_core::finding::Finding;
use pyscythe_core::report::Report;

/// Writes one line per finding followed by a summary.
pub(crate) fn human(report: &Report, out: &mut impl Write) -> std::io::Result<()> {
    for finding in &report.findings {
        writeln!(out, "{}", line_for(finding))?;
    }

    if report.is_clean() {
        writeln!(
            out,
            "No dead code found in {} files ({} symbols checked).",
            report.summary.files_scanned, report.summary.symbols_checked
        )
    } else {
        writeln!(
            out,
            "\n{} finding(s) in {} files ({} symbols checked).",
            report.summary.findings, report.summary.files_scanned, report.summary.symbols_checked
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
