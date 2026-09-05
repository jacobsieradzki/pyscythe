//! Finds module-level definitions that nothing refers to.

use crate::finding::{Confidence, Finding, Rule};
use crate::index::{CodebaseIndex, Reference};
use crate::keep::{KeepContext, Policy};
use crate::manifest::Manifest;
use crate::report::{Report, ReportKind, Summary};
use crate::symbol::Symbol;

/// Runs the dead-code analysis over every file in `index`.
///
/// Unreferenced symbols that `policy` keeps, such as route handlers or entry
/// points named in `manifest`, are counted but not reported.
#[must_use]
pub fn analyze(index: &dyn CodebaseIndex, policy: &Policy, manifest: &Manifest) -> Report {
    let mut findings = Vec::new();
    let mut symbols_checked = 0;
    let mut symbols_kept = 0;

    for file in index.files() {
        for symbol in index.symbols(file.id) {
            let Some(rule) = candidate_rule(&symbol) else {
                continue;
            };
            symbols_checked += 1;

            if is_used(&symbol, &index.references(&symbol)) {
                continue;
            }

            let context = KeepContext {
                symbol: &symbol,
                file,
                manifest,
            };
            if policy.keep_reason(context).is_some() {
                symbols_kept += 1;
                continue;
            }

            findings.push(Finding {
                rule,
                path: file.path.clone(),
                module: file.module.clone(),
                position: index.position(file.id, symbol.name_span.start()),
                confidence: confidence_for(&symbol),
                message: format!("{} `{}` is never used", rule.noun(), symbol.name.as_str()),
                symbol: symbol.name,
            });
        }
    }

    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.position.cmp(&b.position)));

    Report {
        schema_version: Report::SCHEMA_VERSION,
        kind: ReportKind::DeadCode,
        summary: Summary {
            files_scanned: index.files().len(),
            symbols_checked,
            symbols_kept,
            findings: findings.len(),
        },
        findings,
    }
}

/// The rule that would fire for `symbol` if it turns out to be unused.
///
/// Only module-level definitions are candidates for now; dunders such as
/// `__all__` or `__version__` are consumed by the interpreter or tooling.
fn candidate_rule(symbol: &Symbol) -> Option<Rule> {
    if !symbol.is_module_level() || symbol.name.is_dunder() {
        return None;
    }
    Rule::for_unused(symbol.kind)
}

/// A symbol is used when any reference lies outside its own definition.
///
/// References inside the definition, such as a recursive call, do not count.
fn is_used(symbol: &Symbol, references: &[Reference]) -> bool {
    references.iter().any(|reference| match *reference {
        Reference::External => true,
        Reference::Internal { file, span } => {
            file != symbol.file || !symbol.full_span.encloses(span)
        }
    })
}

fn confidence_for(symbol: &Symbol) -> Confidence {
    if symbol.name.is_private() {
        Confidence::High
    } else {
        Confidence::Medium
    }
}

#[cfg(test)]
mod tests {
    use super::analyze;
    use crate::finding::{Confidence, Rule};
    use crate::keep::Policy;
    use crate::manifest::{EntryPoint, EntryPointKind, Manifest};
    use crate::report::Report;
    use crate::source::ModulePath;
    use crate::symbol::{SymbolKind, SymbolName};
    use crate::testing::FakeIndex;

    fn analyze_without_plugins(index: &FakeIndex) -> Report {
        analyze(index, &Policy::none(), &Manifest::empty())
    }

    #[test]
    fn reports_a_module_level_function_with_no_references() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/helpers.py", "pkg.helpers");
        index.add_symbol(file, "orphan", SymbolKind::Function);

        let report = analyze_without_plugins(&index);

        let names: Vec<_> = report.findings.iter().map(|f| f.symbol.as_str()).collect();
        assert_eq!(names, ["orphan"]);
        assert_eq!(report.findings[0].rule, Rule::UnusedFunction);
        assert_eq!(report.summary.symbols_checked, 1);
    }

    #[test]
    fn a_reference_from_another_file_counts_as_use() {
        let mut index = FakeIndex::new();
        let helpers = index.add_file("/proj/pkg/helpers.py", "pkg.helpers");
        let app = index.add_file("/proj/pkg/app.py", "pkg.app");
        let helper = index.add_symbol(helpers, "helper", SymbolKind::Function);
        index.add_reference(helper, app);

        let report = analyze_without_plugins(&index);

        assert!(
            report.is_clean(),
            "expected no findings, got {:?}",
            report.findings
        );
    }

    #[test]
    fn a_reference_inside_the_definition_itself_does_not_count() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/rec.py", "pkg.rec");
        let recursive = index.add_symbol(file, "recurse", SymbolKind::Function);
        index.add_reference_inside_own_body(recursive);

        let report = analyze_without_plugins(&index);

        assert_eq!(report.findings.len(), 1);
    }

    #[test]
    fn external_references_count_as_use() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/api.py", "pkg.api");
        let symbol = index.add_symbol(file, "handler", SymbolKind::Function);
        index.add_external_reference(symbol);

        assert!(analyze_without_plugins(&index).is_clean());
    }

    #[test]
    fn methods_and_dunders_are_not_candidates() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/m.py", "pkg.m");
        let class = index.add_symbol(file, "Widget", SymbolKind::Class);
        index.add_nested_symbol(file, class, "render", SymbolKind::Method);
        index.add_symbol(file, "__all__", SymbolKind::Variable);
        index.add_reference(class, file);

        let report = analyze_without_plugins(&index);

        assert!(report.is_clean());
        assert_eq!(report.summary.symbols_checked, 1);
    }

    #[test]
    fn private_names_are_high_confidence_and_public_names_medium() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/c.py", "pkg.c");
        index.add_symbol(file, "_hidden", SymbolKind::Function);
        index.add_symbol(file, "visible", SymbolKind::Function);

        let report = analyze_without_plugins(&index);

        let confidences: Vec<_> = report
            .findings
            .iter()
            .map(|f| (f.symbol.as_str(), f.confidence))
            .collect();
        assert_eq!(
            confidences,
            [
                ("_hidden", Confidence::High),
                ("visible", Confidence::Medium)
            ]
        );
    }

    #[test]
    fn findings_are_sorted_by_path_then_position() {
        let mut index = FakeIndex::new();
        let zeta = index.add_file("/proj/pkg/zeta.py", "pkg.zeta");
        let alpha = index.add_file("/proj/pkg/alpha.py", "pkg.alpha");
        index.add_symbol(zeta, "z", SymbolKind::Function);
        index.add_symbol(alpha, "a2", SymbolKind::Function);
        index.add_symbol(alpha, "a1", SymbolKind::Function);

        let report = analyze_without_plugins(&index);

        let order: Vec<_> = report.findings.iter().map(|f| f.symbol.as_str()).collect();
        assert_eq!(order, ["a2", "a1", "z"]);
    }

    #[test]
    fn kept_symbols_are_counted_but_not_reported() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/routes.py", "pkg.routes");
        index.add_decorated_symbol(file, "get_user", SymbolKind::Function, &["router.get"]);
        index.add_symbol(file, "orphan", SymbolKind::Function);

        let report = analyze(&index, &Policy::builtin(), &Manifest::empty());

        let names: Vec<_> = report.findings.iter().map(|f| f.symbol.as_str()).collect();
        assert_eq!(names, ["orphan"]);
        assert_eq!(report.summary.symbols_kept, 1);
        assert_eq!(report.summary.symbols_checked, 2);
    }

    #[test]
    fn entry_points_from_the_manifest_are_kept() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/cli.py", "pkg.cli");
        index.add_symbol(file, "main", SymbolKind::Function);
        let manifest = Manifest {
            entry_points: vec![EntryPoint {
                kind: EntryPointKind::Script,
                module: ModulePath::new("pkg.cli"),
                attribute: Some(SymbolName::new("main")),
            }],
        };

        let report = analyze(&index, &Policy::builtin(), &manifest);

        assert!(report.is_clean());
        assert_eq!(report.summary.symbols_kept, 1);
    }
}
