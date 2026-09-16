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
use crate::report::{FileHealth, Grade, HealthSummary, PackageHealth, Report, ReportKind, Summary};
use crate::source::{ModulePath, SourceFile};
use crate::tokens::{CloneMode, CloneToken};

/// Runs the health analysis over every file in `index`.
#[must_use]
pub fn analyze(index: &dyn CodebaseIndex, thresholds: &HealthThresholds) -> Report {
    let mut findings = Vec::new();
    let mut weighted_penalty: u64 = 0;
    let mut total_weight: u64 = 0;
    let mut functions = 0;
    let mut max_cyclomatic = 0;
    let mut max_cognitive = 0;
    let mut files_health = Vec::new();
    let mut package_totals: std::collections::BTreeMap<String, (u64, u64, usize, usize)> =
        std::collections::BTreeMap::new();

    for file in index.files() {
        let metrics = index.function_metrics(file.id);
        let mut file_penalty: u64 = 0;
        let mut file_weight: u64 = 0;
        let mut hotspots = 0;
        for function in &metrics {
            functions += 1;
            max_cyclomatic = max_cyclomatic.max(function.cyclomatic);
            max_cognitive = max_cognitive.max(function.cognitive);

            let weight = u64::from(function.lines.max(1));
            file_penalty += u64::from(penalty(function, thresholds)) * weight;
            file_weight += weight;

            if is_hotspot(function, thresholds) {
                hotspots += 1;
                findings.push(hotspot_finding(index, file, function));
            }
        }
        weighted_penalty += file_penalty;
        total_weight += file_weight;
        if let Some(package) = package_of(file)
            && !metrics.is_empty()
        {
            let totals = package_totals.entry(package).or_default();
            totals.0 += file_penalty;
            totals.1 += file_weight;
            totals.2 += metrics.len();
            totals.3 += hotspots;
        }
        if !metrics.is_empty() {
            files_health.push(FileHealth {
                path: file.path.clone(),
                module: file.module.clone(),
                score: score_from(file_penalty, file_weight),
                maintainability: maintainability_index(
                    &index.clone_tokens(file.id, CloneMode::Strict),
                    &metrics,
                ),
                functions: metrics.len(),
                hotspots,
            });
        }
    }

    sort_worst_first(&mut findings, &mut files_health);
    let packages = package_health(package_totals);
    let score = score_from(weighted_penalty, total_weight);

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
                worst_files: files_health,
                packages,
            }),
            duplication: None,
        },
        findings,
        kept: Vec::new(),
    }
}

/// The second-level package a file belongs to, such as `app.services`.
fn package_of(file: &SourceFile) -> Option<String> {
    let module = file.module.as_ref()?;
    let mut segments = module.as_str().split('.');
    let (first, second) = (segments.next()?, segments.next()?);
    Some(format!("{first}.{second}"))
}

fn hotspot_finding(
    index: &dyn CodebaseIndex,
    file: &SourceFile,
    metrics: &FunctionMetrics,
) -> Finding {
    Finding {
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
    }
}

/// 100 minus the length-weighted average penalty; 100 when nothing was measured.
fn score_from(weighted_penalty: u64, total_weight: u64) -> u8 {
    weighted_penalty
        .checked_div(total_weight)
        .map_or(100, |average| {
            u8::try_from(100u64.saturating_sub(average)).unwrap_or(0)
        })
}

/// The maintainability index as radon scales it: 171 minus terms for
/// Halstead volume, total cyclomatic complexity, and lines, normalised to 0..=100.
///
/// Halstead operands are names and literals; everything else that is not
/// layout is an operator.
#[must_use]
pub fn maintainability_index(tokens: &[CloneToken], functions: &[FunctionMetrics]) -> u8 {
    let mut length: u64 = 0;
    let mut distinct: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for token in tokens {
        if matches!(token.text.as_str(), "\\n" | "\\t" | "\\d") {
            continue;
        }
        length += 1;
        distinct.insert(token.text.as_str());
    }
    let vocabulary = distinct.len().max(1);
    // Halstead volume: N * log2(n). Both counts are far below 2^53.
    #[expect(
        clippy::cast_precision_loss,
        reason = "token counts are small enough to be exact as f64"
    )]
    let volume = (length as f64) * (vocabulary as f64).log2();
    let cyclomatic: u32 = functions.iter().map(|f| f.cyclomatic).sum();
    let lines: u32 = functions.iter().map(|f| f.lines).sum::<u32>().max(1);
    let raw = 16.2f64.mul_add(
        -f64::from(lines).ln(),
        0.23f64.mul_add(
            -f64::from(cyclomatic),
            5.2f64.mul_add(-volume.max(1.0).ln(), 171.0),
        ),
    );
    let scaled = (raw * 100.0 / 171.0).clamp(0.0, 100.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=100 before rounding"
    )]
    let index = scaled.round() as u8;
    index
}

/// Whether the function is worth a reader's attention.
///
/// A long run of assertions counts a branch each and can reach a cyclomatic
/// complexity in the hundreds, but there is nothing to hold in your head: the
/// function is flat. Cognitive complexity says so by staying at zero, and a
/// flat function is not a hotspot however many branches it has.
const fn is_hotspot(metrics: &FunctionMetrics, t: &HealthThresholds) -> bool {
    if metrics.cognitive == 0 {
        return false;
    }
    metrics.cyclomatic > t.max_cyclomatic || metrics.cognitive > t.max_cognitive
}

/// Worst first, so the top of the report is where the effort should go;
/// only the ten worst files are worth listing.
fn sort_worst_first(findings: &mut [Finding], files_health: &mut Vec<FileHealth>) {
    findings.sort_by(|a, b| {
        cognitive_of(b)
            .cmp(&cognitive_of(a))
            .then(a.path.cmp(&b.path))
            .then(a.position.cmp(&b.position))
    });
    files_health.sort_by(|a, b| {
        a.score
            .cmp(&b.score)
            .then(a.maintainability.cmp(&b.maintainability))
            .then(a.path.cmp(&b.path))
    });
    files_health.truncate(10);
}

/// Per-package scores from (penalty, weight, functions, hotspots) totals.
fn package_health(
    totals: std::collections::BTreeMap<String, (u64, u64, usize, usize)>,
) -> Vec<PackageHealth> {
    let mut packages: Vec<PackageHealth> = totals
        .into_iter()
        .map(
            |(package, (penalty, weight, functions, hotspots))| PackageHealth {
                package: ModulePath::new(package),
                score: score_from(penalty, weight),
                functions,
                hotspots,
            },
        )
        .collect();
    packages.sort_by(|a, b| a.score.cmp(&b.score).then(a.package.cmp(&b.package)));
    packages
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
    fn a_flat_run_of_assertions_is_not_a_complexity_hotspot() {
        use super::is_hotspot;
        use crate::metrics::FunctionMetrics;
        use crate::source::{ByteOffset, ByteSpan};
        use crate::symbol::SymbolName;

        let metrics = |cyclomatic: u32, cognitive: u32| FunctionMetrics {
            name: SymbolName::new("f"),
            owner: None,
            name_span: ByteSpan::new(ByteOffset::new(0), ByteOffset::new(1)),
            lines: 340,
            parameters: 1,
            cyclomatic,
            cognitive,
            max_nesting: 0,
        };
        let thresholds = HealthThresholds::default();
        assert!(
            !is_hotspot(&metrics(203, 0), &thresholds),
            "202 assertions in a row are tedious, not complex"
        );
        assert!(
            is_hotspot(&metrics(12, 9), &thresholds),
            "real branching still counts"
        );
    }

    #[test]
    fn a_project_of_simple_functions_scores_one_hundred() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/a.py", "pkg.a");
        index.add_function_metrics(file, "tidy", 3, 4, 20, 2);

        let report = analyze(&index, &HealthThresholds::default());

        let health = report.summary.health.as_ref().expect("health summary");
        assert_eq!(
            (health.score, health.grade, health.functions),
            (100, Grade::A, 1)
        );
        assert!(report.is_clean());
        assert_eq!(health.worst_files.len(), 1);
        assert_eq!(health.worst_files[0].score, 100);
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
        let health = report.summary.health.as_ref().expect("health summary");
        assert_eq!(health.packages.len(), 1);
        assert_eq!(health.packages[0].package.as_str(), "pkg.a");
        assert_eq!(health.packages[0].hotspots, 2);
        assert!(health.score < 60, "score was {}", health.score);
        assert_eq!(health.grade, Grade::F);
        assert_eq!((health.max_cyclomatic, health.max_cognitive), (30, 60));
        assert_eq!(health.worst_files[0].hotspots, 2);
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
    fn maintainability_falls_with_volume_and_complexity() {
        use crate::metrics::FunctionMetrics;
        use crate::source::{ByteOffset, ByteSpan, Line};
        use crate::symbol::SymbolName;
        use crate::tokens::{CloneToken, Nesting};
        let token = |text: &str| CloneToken {
            text: text.to_owned(),
            line: Line::from_one_based(1).unwrap(),
            nesting: Nesting::Statement,
        };
        let function = |cyclomatic: u32, lines: u32| FunctionMetrics {
            name: SymbolName::new("f"),
            owner: None,
            name_span: ByteSpan::new(ByteOffset::new(0), ByteOffset::new(1)),
            lines,
            parameters: 1,
            cyclomatic,
            cognitive: 0,
            max_nesting: 0,
        };
        let tiny =
            super::maintainability_index(&[token("x"), token("="), token("1")], &[function(1, 2)]);
        let big_tokens: Vec<_> = (0..2000).map(|i| token(&format!("name{i}"))).collect();
        let big = super::maintainability_index(&big_tokens, &[function(40, 400)]);
        assert!(tiny > big, "{tiny} vs {big}");
        assert!(tiny >= 80);
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
