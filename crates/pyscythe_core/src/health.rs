//! Complexity hotspots and a single health score.
//!
//! The score starts at 100 for every function and loses points for each unit
//! over a threshold; functions are weighted by length so one long tangled
//! function costs more than a short one. Default thresholds are the common
//! ones: cyclomatic 10, cognitive 15, 50 lines, 6 parameters.

use crate::config::HealthThresholds;
use crate::finding::{Confidence, Detail, Finding, Rule};
use crate::index::CodebaseIndex;
use crate::metrics::FunctionMetrics;
use crate::report::{Grade, HealthSummary, Report, ReportKind, Summary};

/// Runs the health analysis over every file in `index`.
#[must_use]
pub fn analyze(index: &dyn CodebaseIndex, thresholds: &HealthThresholds) -> Report {
    let mut findings = Vec::new();
    let mut weighted_penalty: u64 = 0;
    let mut total_weight: u64 = 0;
    let mut functions = 0;
    let mut max_cyclomatic = 0;
    let mut max_cognitive = 0;

    for file in index.files() {
        for metrics in index.function_metrics(file.id) {
            functions += 1;
            max_cyclomatic = max_cyclomatic.max(metrics.cyclomatic);
            max_cognitive = max_cognitive.max(metrics.cognitive);

            let weight = u64::from(metrics.lines.max(1));
            weighted_penalty += u64::from(penalty(&metrics, thresholds)) * weight;
            total_weight += weight;

            if is_hotspot(&metrics, thresholds) {
                findings.push(Finding {
                    rule: Rule::ComplexFunction,
                    path: file.path.clone(),
                    module: file.module.clone(),
                    position: index.position(file.id, metrics.name_span.start()),
                    confidence: Confidence::High,
                    message: format!(
                        "function `{}` has cyclomatic complexity {} and cognitive complexity {} over {} lines",
                        metrics.qualified_name(),
                        metrics.cyclomatic,
                        metrics.cognitive,
                        metrics.lines
                    ),
                    detail: Detail::Metrics {
                        function: metrics.qualified_name(),
                        cyclomatic: metrics.cyclomatic,
                        cognitive: metrics.cognitive,
                        lines: metrics.lines,
                        parameters: metrics.parameters,
                        max_nesting: metrics.max_nesting,
                    },
                });
            }
        }
    }

    // Worst first, so the top of the report is where the effort should go.
    findings.sort_by(|a, b| {
        cognitive_of(b)
            .cmp(&cognitive_of(a))
            .then(a.path.cmp(&b.path))
            .then(a.position.cmp(&b.position))
    });

    let score = weighted_penalty
        .checked_div(total_weight)
        .map_or(100, |average_penalty| {
            u8::try_from(100u64.saturating_sub(average_penalty)).unwrap_or(0)
        });

    Report {
        schema_version: Report::SCHEMA_VERSION,
        kind: ReportKind::Health,
        summary: Summary {
            files_scanned: index.files().len(),
            symbols_checked: functions,
            symbols_kept: 0,
            symbols_ignored: 0,
            suppressed: 0,
            baselined: 0,
            findings: findings.len(),
            changed_files: None,
            health: Some(HealthSummary {
                score,
                grade: Grade::for_score(score),
                functions,
                max_cyclomatic,
                max_cognitive,
            }),
            duplication: None,
        },
        findings,
        kept: Vec::new(),
    }
}

const fn is_hotspot(metrics: &FunctionMetrics, t: &HealthThresholds) -> bool {
    metrics.cyclomatic > t.max_cyclomatic || metrics.cognitive > t.max_cognitive
}

/// Points a function loses, capped at 100.
fn penalty(metrics: &FunctionMetrics, t: &HealthThresholds) -> u32 {
    let over_cyclomatic = metrics.cyclomatic.saturating_sub(t.max_cyclomatic) * 5;
    let over_cognitive = metrics.cognitive.saturating_sub(t.max_cognitive) * 3;
    let over_lines = metrics.lines.saturating_sub(t.max_lines) / 2;
    let over_parameters = metrics.parameters.saturating_sub(t.max_parameters) * 5;
    (over_cyclomatic + over_cognitive + over_lines + over_parameters).min(100)
}

const fn cognitive_of(finding: &Finding) -> u32 {
    match finding.detail {
        Detail::Metrics { cognitive, .. } => cognitive,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::analyze;
    use crate::config::HealthThresholds;
    use crate::finding::Rule;
    use crate::report::Grade;
    use crate::testing::FakeIndex;

    #[test]
    fn a_project_of_simple_functions_scores_one_hundred() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/a.py", "pkg.a");
        index.add_function_metrics(file, "tidy", 3, 4, 20, 2);

        let report = analyze(&index, &HealthThresholds::default());

        let health = report.summary.health.expect("health summary");
        assert_eq!(
            (health.score, health.grade, health.functions),
            (100, Grade::A, 1)
        );
        assert!(report.is_clean());
    }

    #[test]
    fn hotspots_are_reported_worst_first_and_lower_the_score() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/a.py", "pkg.a");
        index.add_function_metrics(file, "tangled", 14, 22, 80, 3);
        index.add_function_metrics(file, "worse", 30, 60, 200, 9);
        index.add_function_metrics(file, "fine", 2, 1, 10, 1);

        let report = analyze(&index, &HealthThresholds::default());

        let names: Vec<_> = report
            .findings
            .iter()
            .map(|f| (f.rule, f.message.split('`').nth(1).unwrap_or("")))
            .collect();
        assert_eq!(
            names,
            [
                (Rule::ComplexFunction, "worse"),
                (Rule::ComplexFunction, "tangled")
            ]
        );
        let health = report.summary.health.expect("health summary");
        assert!(health.score < 60, "score was {}", health.score);
        assert_eq!(health.grade, Grade::F);
        assert_eq!((health.max_cyclomatic, health.max_cognitive), (30, 60));
    }

    #[test]
    fn thresholds_are_configurable() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/a.py", "pkg.a");
        index.add_function_metrics(file, "busy", 8, 12, 30, 2);
        let strict = HealthThresholds {
            max_cyclomatic: 5,
            max_cognitive: 5,
            ..HealthThresholds::default()
        };

        assert!(analyze(&index, &HealthThresholds::default()).is_clean());
        assert_eq!(analyze(&index, &strict).findings.len(), 1);
    }

    #[test]
    fn a_project_without_functions_is_healthy() {
        let mut index = FakeIndex::new();
        index.add_file("/proj/pkg/empty.py", "pkg.empty");
        let health = analyze(&index, &HealthThresholds::default())
            .summary
            .health
            .expect("health summary");
        assert_eq!(health.score, 100);
        assert_eq!(health.functions, 0);
    }
}
