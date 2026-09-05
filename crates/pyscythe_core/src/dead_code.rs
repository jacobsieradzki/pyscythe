//! Finds module-level definitions and whole files that nothing refers to.

use std::collections::BTreeSet;

use crate::config::Config;
use crate::finding::{Confidence, Detail, Finding, Rule};
use crate::index::{CodebaseIndex, Reference};
use crate::keep::{KeepContext, Policy};
use crate::manifest::Manifest;
use crate::report::{Report, ReportKind, Summary};
use crate::source::{FileId, MainGuard, SourceFile};
use crate::symbol::Symbol;

/// Files that are run or loaded by convention rather than imported.
const ROOT_FILE_NAMES: &[&str] = &[
    "__init__.py",
    "__main__.py",
    "conftest.py",
    "setup.py",
    "manage.py",
    "wsgi.py",
    "asgi.py",
    "noxfile.py",
    "tasks.py",
];

/// Runs the dead-code analysis over every file in `index`.
///
/// Unreferenced symbols that `policy` keeps, such as route handlers or entry
/// points named in `manifest`, are counted but not reported. A file that no
/// other file imports and that nothing runs is reported instead of its symbols.
#[must_use]
pub fn analyze(
    index: &dyn CodebaseIndex,
    policy: &Policy,
    manifest: &Manifest,
    config: &Config,
) -> Report {
    let mut findings = Vec::new();
    let mut symbols_checked = 0;
    let mut symbols_kept = 0;
    let mut symbols_ignored = 0;
    let mut files_with_kept_symbols: BTreeSet<FileId> = BTreeSet::new();

    for file in index.files() {
        for symbol in index.symbols(file.id) {
            let Some(rule) = candidate_rule(&symbol) else {
                continue;
            };
            if config.ignore_names.matches(symbol.name.as_str()) {
                symbols_ignored += 1;
                continue;
            }
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
                files_with_kept_symbols.insert(file.id);
                continue;
            }

            findings.push(Finding {
                rule,
                path: file.path.clone(),
                module: file.module.clone(),
                position: index.position(file.id, symbol.name_span.start()),
                confidence: confidence_for(&symbol),
                message: format!("{} `{}` is never used", rule.noun(), symbol.name.as_str()),
                detail: Detail::Symbol {
                    symbol: symbol.name,
                },
            });
        }
    }

    let unused_files = unused_files(index, manifest, &files_with_kept_symbols);
    findings.retain(|finding| !unused_files.iter().any(|file| file.path == finding.path));
    findings.extend(unused_files.into_iter().map(|file| Finding {
        rule: Rule::UnusedFile,
        path: file.path.clone(),
        module: file.module.clone(),
        position: None,
        confidence: Confidence::Medium,
        message: format!(
                "file `{}` is never imported or run",
                file.module
                    .as_ref()
                    .map_or_else(|| file.file_name().to_owned(), |m| m.as_str().to_owned())
            ),
        detail: Detail::File,
    }));

    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.position.cmp(&b.position)));

    Report {
        schema_version: Report::SCHEMA_VERSION,
        kind: ReportKind::DeadCode,
        summary: Summary {
            files_scanned: index.files().len(),
            symbols_checked,
            symbols_kept,
            symbols_ignored,
            findings: findings.len(),
        },
        findings,
    }
}

/// Files nothing imports and nothing runs.
fn unused_files<'a>(
    index: &'a dyn CodebaseIndex,
    manifest: &Manifest,
    files_with_kept_symbols: &BTreeSet<FileId>,
) -> Vec<&'a SourceFile> {
    let files = index.files();
    let imported: BTreeSet<FileId> = files
        .iter()
        .flat_map(|file| {
            index
                .imports(file.id)
                .into_iter()
                .map(|import| import.target)
                .filter(move |target| *target != file.id)
        })
        .collect();

    files
        .iter()
        .filter(|file| !imported.contains(&file.id))
        .filter(|file| !is_root_file(file, manifest, files_with_kept_symbols))
        .collect()
}

fn is_root_file(
    file: &SourceFile,
    manifest: &Manifest,
    files_with_kept_symbols: &BTreeSet<FileId>,
) -> bool {
    let name = file.file_name();
    file.main_guard == MainGuard::Present
        || ROOT_FILE_NAMES.contains(&name)
        || is_test_file(name)
        || files_with_kept_symbols.contains(&file.id)
        || file
            .module
            .as_ref()
            .is_some_and(|module| manifest.entry_points.iter().any(|ep| &ep.module == module))
}

fn is_test_file(name: &str) -> bool {
    name.strip_suffix(".py")
        .is_some_and(|stem| stem.starts_with("test_") || stem.ends_with("_test"))
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
    use crate::config::{Config, NamePatterns};
    use crate::finding::{Confidence, Rule};
    use crate::index::ImportKind;
    use crate::keep::Policy;
    use crate::manifest::{EntryPoint, EntryPointKind, Manifest};
    use crate::report::Report;
    use crate::source::ModulePath;
    use crate::symbol::{SymbolKind, SymbolName};
    use crate::testing::FakeIndex;

    fn analyze_without_plugins(index: &FakeIndex) -> Report {
        analyze(
            index,
            &Policy::none(),
            &Manifest::empty(),
            &Config::default(),
        )
    }

    fn symbol_names(report: &Report) -> Vec<&str> {
        report
            .findings
            .iter()
            .filter_map(|f| f.symbol().map(SymbolName::as_str))
            .collect()
    }

    /// Marks `file` as imported by another file so it is not reported as unused.
    fn import_from_elsewhere(index: &mut FakeIndex, file: crate::source::FileId) {
        let importer = index.add_file("/proj/pkg/__init__.py", "pkg");
        index.add_import(importer, file, ImportKind::Runtime);
    }

    #[test]
    fn reports_a_module_level_function_with_no_references() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/helpers.py", "pkg.helpers");
        index.add_symbol(file, "orphan", SymbolKind::Function);
        import_from_elsewhere(&mut index, file);

        let report = analyze_without_plugins(&index);

        assert_eq!(symbol_names(&report), ["orphan"]);
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
        index.add_import(app, helpers, ImportKind::Runtime);
        import_from_elsewhere(&mut index, app);

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
        import_from_elsewhere(&mut index, file);

        assert_eq!(symbol_names(&analyze_without_plugins(&index)), ["recurse"]);
    }

    #[test]
    fn external_references_count_as_use() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/api.py", "pkg.api");
        let symbol = index.add_symbol(file, "handler", SymbolKind::Function);
        index.add_external_reference(symbol);
        import_from_elsewhere(&mut index, file);

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
        import_from_elsewhere(&mut index, file);

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
        import_from_elsewhere(&mut index, file);

        let report = analyze_without_plugins(&index);

        let confidences: Vec<_> = report
            .findings
            .iter()
            .map(|f| (f.symbol().unwrap().as_str(), f.confidence))
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
        import_from_elsewhere(&mut index, zeta);
        index.add_import(zeta, alpha, ImportKind::Runtime);

        assert_eq!(
            symbol_names(&analyze_without_plugins(&index)),
            ["a2", "a1", "z"]
        );
    }

    #[test]
    fn kept_symbols_are_counted_but_not_reported() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/routes.py", "pkg.routes");
        index.add_decorated_symbol(file, "get_user", SymbolKind::Function, &["router.get"]);
        index.add_symbol(file, "orphan", SymbolKind::Function);

        let report = analyze(
            &index,
            &Policy::builtin(),
            &Manifest::empty(),
            &Config::default(),
        );

        assert_eq!(symbol_names(&report), ["orphan"]);
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

        let report = analyze(&index, &Policy::builtin(), &manifest, &Config::default());

        assert!(report.is_clean());
        assert_eq!(report.summary.symbols_kept, 1);
    }

    #[test]
    fn ignored_names_are_skipped_and_counted() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/old.py", "pkg.old");
        index.add_symbol(file, "legacy_handler", SymbolKind::Function);
        index.add_symbol(file, "handler", SymbolKind::Function);
        import_from_elsewhere(&mut index, file);
        let config = Config {
            ignore_names: NamePatterns::parse(["legacy_*"]).unwrap(),
            ..Config::default()
        };

        let report = analyze(&index, &Policy::none(), &Manifest::empty(), &config);

        assert_eq!(symbol_names(&report), ["handler"]);
        assert_eq!(report.summary.symbols_ignored, 1);
    }

    #[test]
    fn a_file_nobody_imports_is_reported_instead_of_its_symbols() {
        let mut index = FakeIndex::new();
        let orphan = index.add_file("/proj/pkg/orphan.py", "pkg.orphan");
        index.add_symbol(orphan, "helper", SymbolKind::Function);
        index.add_symbol(orphan, "Other", SymbolKind::Class);

        let report = analyze_without_plugins(&index);

        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].rule, Rule::UnusedFile);
        assert_eq!(
            report.findings[0].message,
            "file `pkg.orphan` is never imported or run"
        );
    }

    #[test]
    fn scripts_packages_tests_and_entry_point_modules_are_not_unused_files() {
        let mut index = FakeIndex::new();
        index.add_script("/proj/scripts/migrate.py", "scripts.migrate");
        index.add_file("/proj/pkg/__init__.py", "pkg");
        index.add_file("/proj/pkg/__main__.py", "pkg.__main__");
        index.add_file("/proj/tests/test_it.py", "tests.test_it");
        index.add_file("/proj/pkg/cli.py", "pkg.cli");
        let manifest = Manifest {
            entry_points: vec![EntryPoint {
                kind: EntryPointKind::Plugin,
                module: ModulePath::new("pkg.cli"),
                attribute: None,
            }],
        };

        let report = analyze(&index, &Policy::none(), &manifest, &Config::default());

        assert!(report.is_clean(), "{:?}", report.findings);
    }

    #[test]
    fn a_file_whose_symbols_a_plugin_keeps_is_a_root() {
        let mut index = FakeIndex::new();
        let routes = index.add_file("/proj/pkg/routes.py", "pkg.routes");
        index.add_decorated_symbol(routes, "get_user", SymbolKind::Function, &["router.get"]);

        let report = analyze(
            &index,
            &Policy::builtin(),
            &Manifest::empty(),
            &Config::default(),
        );

        assert!(report.is_clean(), "{:?}", report.findings);
    }

    #[test]
    fn type_only_and_deferred_imports_still_make_a_file_used() {
        let mut index = FakeIndex::new();
        let types = index.add_file("/proj/pkg/types.py", "pkg.types");
        let app = index.add_file("/proj/pkg/app.py", "pkg.app");
        index.add_import(app, types, ImportKind::TypeOnly);
        import_from_elsewhere(&mut index, app);

        assert!(analyze_without_plugins(&index).is_clean());
    }
}
