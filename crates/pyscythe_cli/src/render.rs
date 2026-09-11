//! Human-readable rendering of reports.

use std::io::Write;

use camino::Utf8Path;
use pyscythe_core::finding::Confidence;

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
    let details = summary_details(report);
    let scope = summary
        .changed_files
        .map_or_else(String::new, |n| format!(" in {n} changed file(s)"));
    if let (ReportKind::Health, Some(health)) = (report.kind, summary.health.as_ref()) {
        return health_summary(report, health, &details, &scope, out);
    }
    let subject = match report.kind {
        ReportKind::DeadCode => "dead code",
        ReportKind::Cycles => "import cycles",
        ReportKind::Health => "hotspots",
        ReportKind::Dupes => "duplicated code",
        ReportKind::Boundaries => "boundary violations",
        ReportKind::Deps => "dependency problems",
    };
    if report.is_clean() {
        writeln!(
            out,
            "No {subject} found in {} files{details}{scope}.",
            summary.files_scanned
        )
    } else {
        writeln!(
            out,
            "\n{} finding(s) in {} files{details}{scope}.",
            summary.findings, summary.files_scanned
        )
    }
}

/// The parenthesised counts after the summary line, per analysis.
fn summary_details(report: &Report) -> String {
    let summary = &report.summary;
    match report.kind {
        ReportKind::DeadCode => {
            let mut parts = vec![format!("{} symbols checked", summary.symbols_checked)];
            if summary.symbols_kept > 0 {
                parts.push(format!("{} kept by plugins", summary.symbols_kept));
            }
            if summary.symbols_ignored > 0 {
                parts.push(format!("{} ignored by config", summary.symbols_ignored));
            }
            if summary.suppressed > 0 {
                parts.push(format!("{} suppressed by comments", summary.suppressed));
            }
            if summary.baselined > 0 {
                parts.push(format!("{} in baseline", summary.baselined));
            }
            format!(" ({})", parts.join(", "))
        }
        ReportKind::Cycles | ReportKind::Boundaries | ReportKind::Deps => String::new(),
        ReportKind::Health => summary.health.as_ref().map_or_else(String::new, |health| {
            format!(
                " ({} functions, max cyclomatic {}, max cognitive {})",
                health.functions, health.max_cyclomatic, health.max_cognitive
            )
        }),
        ReportKind::Dupes => summary.duplication.map_or_else(String::new, |d| {
            format!(
                " ({}.{}% duplicated: {} of {} lines)",
                d.percent_tenths / 10,
                d.percent_tenths % 10,
                d.duplicated_lines,
                d.total_lines
            )
        }),
    }
}

/// The files needing attention and the score line.
fn health_summary(
    report: &Report,
    health: &pyscythe_core::report::HealthSummary,
    details: &str,
    scope: &str,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let worst: Vec<&pyscythe_core::report::FileHealth> = health
        .worst_files
        .iter()
        .filter(|file| file.score < 100 || file.maintainability < 65)
        .take(5)
        .collect();
    if !worst.is_empty() {
        writeln!(out, "\nFiles needing attention:")?;
        for file in worst {
            let name = file
                .module
                .as_ref()
                .map_or_else(|| file.path.to_string(), |m| m.as_str().to_owned());
            writeln!(
                out,
                "  {name}: score {}, maintainability {}, {} hotspot(s) in {} function(s)",
                file.score, file.maintainability, file.hotspots, file.functions
            )?;
        }
    }
    writeln!(
        out,
        "\nHealth score {}/100 ({}) across {} files{details}; {} hotspot(s){scope}.",
        health.score,
        health.grade.letter(),
        report.summary.files_scanned,
        report.summary.findings
    )
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

/// Project-relative display path.
fn relative<'a>(path: &'a Utf8Path, root: &Utf8Path) -> &'a Utf8Path {
    path.strip_prefix(root).unwrap_or(path)
}

/// One `::warning`/`::notice` workflow command per finding.
pub(crate) fn github_annotations(
    report: &Report,
    root: &Utf8Path,
    out: &mut impl Write,
) -> std::io::Result<()> {
    for finding in &report.findings {
        let level = match finding.confidence {
            Confidence::High | Confidence::Medium => "warning",
            Confidence::Low => "notice",
        };
        let file = relative(&finding.path, root);
        let position = finding.position.map_or_else(String::new, |p| {
            format!(",line={},col={}", p.line.get(), p.column.get())
        });
        writeln!(
            out,
            "::{level} file={file}{position},title=pyscythe {}::{}",
            finding.rule.code(),
            finding.message
        )?;
    }
    Ok(())
}

/// A table suitable for a pull request comment.
pub(crate) fn markdown(
    report: &Report,
    root: &Utf8Path,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let summary = &report.summary;
    if report.is_clean() {
        return writeln!(
            out,
            "**pyscythe**: no findings in {} files.",
            summary.files_scanned
        );
    }
    writeln!(
        out,
        "**pyscythe**: {} finding(s) in {} files.\n",
        summary.findings, summary.files_scanned
    )?;
    writeln!(out, "| Rule | Location | Finding | Confidence |")?;
    writeln!(out, "| --- | --- | --- | --- |")?;
    for finding in &report.findings {
        let file = relative(&finding.path, root);
        let location = finding
            .position
            .map_or_else(|| file.to_string(), |p| format!("{file}:{}", p.line.get()));
        let confidence = match finding.confidence {
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
        };
        writeln!(
            out,
            "| `{}` | `{location}` | {} | {confidence} |",
            finding.rule.code(),
            finding.message.replace('|', "\\|")
        )?;
    }
    Ok(())
}
