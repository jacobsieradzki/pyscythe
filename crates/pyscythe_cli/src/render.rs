//! Human-readable rendering of reports.

use std::io::Write;

use pyscythe_core::finding::Finding;
use pyscythe_core::report::Report;

/// Writes one line per finding followed by a summary.
pub(crate) fn human(report: &Report, out: &mut impl Write) -> std::io::Result<()> {
    for finding in &report.findings {
        writeln!(out, "{}", line_for(finding))?;
    }

    let summary = &report.summary;
    let kept = if summary.symbols_kept == 0 {
        String::new()
    } else {
        format!(", {} kept by plugins", summary.symbols_kept)
    };
    if report.is_clean() {
        writeln!(
            out,
            "No dead code found in {} files ({} symbols checked{kept}).",
            summary.files_scanned, summary.symbols_checked
        )
    } else {
        writeln!(
            out,
            "\n{} finding(s) in {} files ({} symbols checked{kept}).",
            summary.findings, summary.files_scanned, summary.symbols_checked
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
