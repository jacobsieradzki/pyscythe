//! Finds module-level definitions and whole files that nothing refers to.

use std::collections::{BTreeMap, BTreeSet};

use crate::config::Config;
use crate::finding::{Confidence, Detail, Finding, Rule};
use crate::index::{
    Ancestry, CodebaseIndex, Inheritance, NameUsage, Reference, Suppression, SuppressionScope,
};
use crate::keep::{KeepContext, KeepReason, PluginName, Policy};
use crate::manifest::Manifest;
use crate::report::{KeptSymbol, Report, ReportKind, Summary};
use crate::source::{Column, FileId, Line, MainGuard, Position, SourceFile};
use crate::symbol::{Symbol, SymbolId, SymbolKind, SymbolName, SymbolScope};

/// Files that are run or loaded by convention rather than imported.
const ROOT_FILE_NAMES: &[&str] = &[
    "__init__.py",
    "__main__.py",
    "conftest.py",
    "conf.py",
    "setup.py",
    "manage.py",
    "wsgi.py",
    "asgi.py",
    "noxfile.py",
    "tasks.py",
];

/// Directories whose files are run directly or built by tooling, never imported.
const ROOT_DIRECTORY_PREFIXES: &[&str] =
    &["bench", "bin", "doc", "example", "sample", "script", "tool"];

/// Whether the file lives under a directory of things that are run or built
/// directly (`docs`, `examples`, `benchmarks`, `scripts`, `bin`, `tools`).
#[must_use]
pub fn is_in_root_directory(file: &SourceFile) -> bool {
    file.path.parent().is_some_and(|dir| {
        dir.components().any(|component| {
            let name = component.as_str();
            ROOT_DIRECTORY_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
        })
    })
}

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
    let mut kept = Vec::new();
    let mut symbols_checked = 0;
    let mut symbols_ignored = 0;
    let mut suppressed = 0;
    let mut files_with_kept_symbols: BTreeSet<FileId> = BTreeSet::new();
    let mut suppressed_files: BTreeSet<FileId> = BTreeSet::new();
    let mut stale_suppressions: Vec<(FileId, Line)> = Vec::new();

    for file in index.files() {
        let symbols = index.symbols(file.id);
        let owner_names: BTreeMap<SymbolId, SymbolName> =
            symbols.iter().map(|s| (s.id, s.name.clone())).collect();
        let suppressions = index.suppressions(file.id);
        if suppressions
            .iter()
            .any(|s| s.scope == SuppressionScope::File)
        {
            suppressed_files.insert(file.id);
        }

        let mut used_suppression_lines: BTreeSet<Line> = BTreeSet::new();
        for symbol in symbols {
            let checker = SymbolCheck {
                index,
                policy,
                manifest,
                config,
                file,
                owner_names: &owner_names,
                suppressions: &suppressions,
            };
            match checker.verdict(symbol) {
                Verdict::NotCandidate => {}
                Verdict::Ignored => symbols_ignored += 1,
                Verdict::Used => symbols_checked += 1,
                Verdict::Suppressed(line) => {
                    symbols_checked += 1;
                    suppressed += 1;
                    used_suppression_lines.insert(line);
                }
                Verdict::Kept(symbol) => {
                    symbols_checked += 1;
                    files_with_kept_symbols.insert(file.id);
                    kept.push(symbol);
                }
                Verdict::Dead(finding) => {
                    symbols_checked += 1;
                    findings.push(finding);
                }
            }
        }
        if !suppressed_files.contains(&file.id) {
            stale_suppressions.extend(
                suppressions
                    .iter()
                    .filter(|s| s.scope != SuppressionScope::File)
                    .filter(|s| !used_suppression_lines.contains(&s.line))
                    .map(|s| (file.id, s.line)),
            );
        }
    }

    findings.extend(stale_suppression_findings(index, stale_suppressions));

    let unused_files = unused_files(index, manifest, &files_with_kept_symbols);
    findings.retain(|finding| !unused_files.iter().any(|file| file.path == finding.path));
    let (suppressed_unused_files, reported_unused_files): (Vec<_>, Vec<_>) = unused_files
        .into_iter()
        .partition(|file| suppressed_files.contains(&file.id));
    suppressed += suppressed_unused_files.len();
    findings.extend(reported_unused_files.into_iter().map(unused_file_finding));

    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.position.cmp(&b.position)));
    kept.sort_by(|a, b| a.path.cmp(&b.path).then(a.position.cmp(&b.position)));

    Report {
        schema_version: Report::SCHEMA_VERSION,
        kind: ReportKind::DeadCode,
        summary: Summary {
            files_scanned: index.files().len(),
            symbols_checked,
            symbols_kept: kept.len(),
            symbols_ignored,
            suppressed,
            baselined: 0,
            findings: findings.len(),
            changed_files: None,
            health: None,
            duplication: None,
        },
        findings,
        kept,
    }
}

/// What the analysis concluded about one symbol.
enum Verdict {
    /// Not something the analysis looks at, such as a dunder or a parameter.
    NotCandidate,
    /// Matched `ignore-names`.
    Ignored,
    /// Something refers to it.
    Used,
    /// Dead, but a `# pyscythe: ignore` comment on this line covers it.
    Suppressed(Line),
    /// Unreferenced, but a plugin or an override keeps it.
    Kept(KeptSymbol),
    /// Unreferenced and nothing keeps it.
    Dead(Finding),
}

/// Everything needed to judge the symbols of one file.
struct SymbolCheck<'a> {
    index: &'a dyn CodebaseIndex,
    policy: &'a Policy,
    manifest: &'a Manifest,
    config: &'a Config,
    file: &'a SourceFile,
    owner_names: &'a BTreeMap<SymbolId, SymbolName>,
    suppressions: &'a [Suppression],
}

impl SymbolCheck<'_> {
    fn verdict(&self, symbol: Symbol) -> Verdict {
        let Some(rule) = candidate_rule(&symbol) else {
            return Verdict::NotCandidate;
        };
        if self.config.ignore_names.matches(symbol.name.as_str()) {
            return Verdict::Ignored;
        }
        if is_used(self.index, &symbol) {
            return Verdict::Used;
        }

        let position = self.index.position(self.file.id, symbol.name_span.start());
        let ancestry = if symbol.kind == SymbolKind::Class {
            self.index.ancestry(&symbol)
        } else {
            Ancestry::unknown()
        };
        let context = KeepContext {
            symbol: &symbol,
            file: self.file,
            manifest: self.manifest,
            ancestry: &ancestry,
            public_modules: &self.config.public_modules,
        };
        let reason = self
            .policy
            .keep_reason(context)
            .or_else(|| override_reason(self.index, &symbol));
        if let Some(reason) = reason {
            return Verdict::Kept(KeptSymbol {
                path: self.file.path.clone(),
                module: self.file.module.clone(),
                symbol: symbol.name,
                position,
                plugin: reason.plugin,
                why: reason.why,
            });
        }

        if let Some(line) = self.suppressing_line(&symbol, rule, position) {
            return Verdict::Suppressed(line);
        }

        let owner = match symbol.scope {
            SymbolScope::Nested { parent } => self.owner_names.get(&parent).cloned(),
            SymbolScope::Module => None,
        };
        let qualified = owner.as_ref().map_or_else(
            || symbol.name.as_str().to_owned(),
            |owner| format!("{}.{}", owner.as_str(), symbol.name.as_str()),
        );
        Verdict::Dead(Finding {
            rule,
            path: self.file.path.clone(),
            module: self.file.module.clone(),
            position,
            confidence: confidence_for(&symbol),
            message: format!("{} `{qualified}` is never used", rule.noun()),
            detail: Detail::Symbol {
                symbol: symbol.name,
                owner,
            },
        })
    }
}

fn stale_suppression_findings(
    index: &dyn CodebaseIndex,
    stale: Vec<(FileId, Line)>,
) -> Vec<Finding> {
    stale
        .into_iter()
        .filter_map(|(file_id, line)| {
            let file = index.file(file_id)?;
            Some(Finding {
                rule: Rule::UnusedSuppression,
                path: file.path.clone(),
                module: file.module.clone(),
                position: Some(Position {
                    line,
                    column: Column::from_one_based(1)?,
                }),
                confidence: Confidence::High,
                message: "suppression comment silences nothing".to_owned(),
                detail: Detail::Comment,
            })
        })
        .collect()
}

fn unused_file_finding(file: &SourceFile) -> Finding {
    Finding {
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
        || is_in_root_directory(file)
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

impl SymbolCheck<'_> {
    /// The line of a comment that silences `rule` for the definition: on its
    /// name line, on the line just above the definition (above any
    /// decorators), or anywhere in the file for `ignore-file`.
    fn suppressing_line(
        &self,
        symbol: &Symbol,
        rule: Rule,
        position: Option<Position>,
    ) -> Option<Line> {
        let name_line = position.map(|p| p.line);
        let line_above = self
            .index
            .position(self.file.id, symbol.full_span.start())
            .and_then(|p| Line::from_one_based(p.line.get().saturating_sub(1)));
        self.suppressions
            .iter()
            .find(|suppression| {
                let covers_rule = match &suppression.scope {
                    SuppressionScope::File => return true,
                    SuppressionScope::AllRules => true,
                    SuppressionScope::Rules(rules) => rules.contains(&rule),
                };
                covers_rule
                    && (Some(suppression.line) == name_line || Some(suppression.line) == line_above)
            })
            .map(|suppression| suppression.line)
    }
}

/// A method that overrides an inherited member may be called by the base
/// class's own code, which never names the subclass.
fn override_reason(index: &dyn CodebaseIndex, symbol: &Symbol) -> Option<KeepReason> {
    let is_member = matches!(symbol.kind, SymbolKind::Method | SymbolKind::Property);
    (is_member && index.inheritance(symbol) == Inheritance::OverridesBase).then_some(KeepReason {
        plugin: PluginName::Python,
        why: "overrides an inherited member",
    })
}

/// The rule that would fire for `symbol` if it turns out to be unused.
///
/// Dunders such as `__all__`, `__init__`, or `__version__` are consumed by the
/// interpreter or tooling. Module-level functions, classes, and variables are
/// candidates; so are methods and properties directly on a class, except the
/// `.setter`/`.deleter` halves of a property, which share the getter's name.
fn candidate_rule(symbol: &Symbol) -> Option<Rule> {
    if symbol.name.is_dunder() {
        return None;
    }
    match symbol.scope {
        SymbolScope::Module => Rule::for_unused(symbol.kind),
        SymbolScope::Nested { .. } => match symbol.kind {
            SymbolKind::Method | SymbolKind::Property if !is_property_accessor(symbol) => {
                Some(Rule::UnusedMethod)
            }
            _ => None,
        },
    }
}

fn is_property_accessor(symbol: &Symbol) -> bool {
    symbol.has_decorator(|d| matches!(d.name.last_segment(), "setter" | "deleter" | "getter"))
}

/// A symbol is used when a reference resolves to it from outside its own
/// definition, or, for methods, when its name is accessed as an attribute
/// anywhere: a call through a base type or an untyped object cannot be
/// resolved to one method but still shows up by name.
fn is_used(index: &dyn CodebaseIndex, symbol: &Symbol) -> bool {
    let referenced = index
        .references(symbol)
        .iter()
        .any(|reference| match *reference {
            Reference::External => true,
            Reference::Internal { file, span } => {
                file != symbol.file || !symbol.full_span.encloses(span)
            }
        });
    if referenced {
        return true;
    }
    matches!(symbol.kind, SymbolKind::Method | SymbolKind::Property)
        && index.attribute_name_usage(&symbol.name) == NameUsage::Used
}

fn confidence_for(symbol: &Symbol) -> Confidence {
    match (symbol.scope, symbol.name.is_private()) {
        (SymbolScope::Module, true) => Confidence::High,
        (SymbolScope::Module, false) | (SymbolScope::Nested { .. }, true) => Confidence::Medium,
        (SymbolScope::Nested { .. }, false) => Confidence::Low,
    }
}

#[cfg(test)]
mod tests {
    use super::analyze;
    use crate::config::{Config, NamePatterns};
    use crate::finding::{Confidence, Rule};
    use crate::index::{Ancestry, ImportKind, SuppressionScope};
    use crate::keep::Policy;
    use crate::manifest::{EntryPoint, EntryPointKind, Manifest};
    use crate::report::Report;
    use crate::source::ModulePath;
    use crate::symbol::{DottedName, SymbolKind, SymbolName};
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
    fn dunders_and_nested_classes_are_not_candidates() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/m.py", "pkg.m");
        let class = index.add_symbol(file, "Widget", SymbolKind::Class);
        index.add_nested_symbol(file, class, "__init__", SymbolKind::Method);
        index.add_nested_symbol(file, class, "Meta", SymbolKind::Class);
        index.add_symbol(file, "__all__", SymbolKind::Variable);
        index.add_reference(class, file);
        import_from_elsewhere(&mut index, file);

        let report = analyze_without_plugins(&index);

        assert!(report.is_clean(), "{:?}", report.findings);
        assert_eq!(report.summary.symbols_checked, 1);
    }

    #[test]
    fn an_unused_method_is_reported_with_its_owner_at_low_confidence() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/m.py", "pkg.m");
        let class = index.add_symbol(file, "Widget", SymbolKind::Class);
        index.add_nested_symbol(file, class, "render", SymbolKind::Method);
        index.add_nested_symbol(file, class, "_prepare", SymbolKind::Method);
        index.add_reference(class, file);
        import_from_elsewhere(&mut index, file);

        let report = analyze_without_plugins(&index);

        let summary: Vec<_> = report
            .findings
            .iter()
            .map(|f| (f.message.as_str(), f.confidence, f.rule))
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "method `Widget.render` is never used",
                    Confidence::Low,
                    Rule::UnusedMethod
                ),
                (
                    "method `Widget._prepare` is never used",
                    Confidence::Medium,
                    Rule::UnusedMethod
                ),
            ]
        );
    }

    #[test]
    fn a_method_whose_name_is_accessed_anywhere_is_used() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/m.py", "pkg.m");
        let class = index.add_symbol(file, "Widget", SymbolKind::Class);
        index.add_nested_symbol(file, class, "render", SymbolKind::Method);
        index.add_reference(class, file);
        index.mark_attribute_name_used("render");
        import_from_elsewhere(&mut index, file);

        assert!(analyze_without_plugins(&index).is_clean());
    }

    #[test]
    fn a_method_overriding_a_base_member_is_kept() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/m.py", "pkg.m");
        let class = index.add_symbol(file, "Store", SymbolKind::Class);
        let override_ =
            index.add_nested_symbol(file, class, "similarity_search", SymbolKind::Method);
        index.mark_overrides_base(override_);
        index.add_reference(class, file);
        import_from_elsewhere(&mut index, file);

        let report = analyze_without_plugins(&index);

        assert!(report.is_clean(), "{:?}", report.findings);
        assert_eq!(report.kept[0].why, "overrides an inherited member");
    }

    #[test]
    fn plugins_see_resolved_ancestry_rather_than_base_names() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/m.py", "pkg.m");
        let local = index.add_decorated_symbol(file, "Child", SymbolKind::Class, &[]);
        let orm = index.add_decorated_symbol(file, "User", SymbolKind::Class, &[]);
        index.set_ancestry(
            local,
            Ancestry::Complete(vec![
                DottedName::new("pkg.m.Base"),
                DottedName::new("builtins.object"),
            ]),
        );
        index.set_ancestry(
            orm,
            Ancestry::Complete(vec![
                DottedName::new("pkg.db.Base"),
                DottedName::new("sqlalchemy.orm.decl_api.DeclarativeBase"),
                DottedName::new("builtins.object"),
            ]),
        );
        import_from_elsewhere(&mut index, file);

        let report = analyze(
            &index,
            &Policy::builtin(),
            &Manifest::empty(),
            &Config::default(),
        );

        assert_eq!(symbol_names(&report), ["Child"]);
        assert_eq!(report.kept[0].symbol.as_str(), "User");
    }

    #[test]
    fn suppression_comments_silence_findings_on_their_line_or_the_line_above() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/m.py", "pkg.m");
        let on_line = index.add_symbol(file, "on_line", SymbolKind::Function);
        let above = index.add_symbol(file, "above", SymbolKind::Function);
        let wrong_rule = index.add_symbol(file, "wrong_rule", SymbolKind::Function);
        index.add_symbol(file, "loud", SymbolKind::Function);
        index.suppress_on_name_line(on_line, SuppressionScope::AllRules);
        index.suppress_on_line_above(above, SuppressionScope::Rules(vec![Rule::UnusedFunction]));
        index.suppress_on_name_line(wrong_rule, SuppressionScope::Rules(vec![Rule::UnusedClass]));
        import_from_elsewhere(&mut index, file);

        let report = analyze_without_plugins(&index);

        assert_eq!(symbol_names(&report), ["wrong_rule", "loud"]);
        assert_eq!(report.summary.suppressed, 2);
        let stale: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.rule == Rule::UnusedSuppression)
            .map(|f| f.position.unwrap().line.get())
            .collect();
        assert_eq!(
            stale,
            [3],
            "the comment naming the wrong rule silences nothing"
        );
    }

    #[test]
    fn a_suppression_on_a_used_symbol_is_reported_as_stale() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/m.py", "pkg.m");
        let used = index.add_symbol(file, "used", SymbolKind::Function);
        index.add_reference(used, file);
        index.suppress_on_name_line(used, SuppressionScope::AllRules);
        import_from_elsewhere(&mut index, file);

        let report = analyze_without_plugins(&index);

        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].rule, Rule::UnusedSuppression);
    }

    #[test]
    fn ignore_file_silences_a_whole_file_including_unused_file() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/legacy.py", "pkg.legacy");
        index.add_symbol(file, "old", SymbolKind::Function);
        index.suppress_file(file);

        let report = analyze_without_plugins(&index);

        assert!(report.is_clean(), "{:?}", report.findings);
        assert_eq!(report.summary.suppressed, 2, "the symbol and the file");
    }

    #[test]
    fn property_setters_are_not_candidates() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/m.py", "pkg.m");
        let class = index.add_symbol(file, "Widget", SymbolKind::Class);
        index.add_decorated_nested_symbol(file, class, "size", SymbolKind::Property, &["property"]);
        index.add_decorated_nested_symbol(
            file,
            class,
            "size",
            SymbolKind::Property,
            &["size.setter"],
        );
        index.add_reference(class, file);
        import_from_elsewhere(&mut index, file);

        let report = analyze_without_plugins(&index);

        assert_eq!(report.findings.len(), 1, "only the getter is a candidate");
        assert_eq!(report.summary.symbols_checked, 2);
    }

    #[test]
    fn kept_symbols_are_listed_with_their_reason() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/routes.py", "pkg.routes");
        index.add_decorated_symbol(file, "get_user", SymbolKind::Function, &["router.get"]);

        let report = analyze(
            &index,
            &Policy::builtin(),
            &Manifest::empty(),
            &Config::default(),
        );

        assert_eq!(report.kept.len(), 1);
        assert_eq!(report.kept[0].symbol.as_str(), "get_user");
        assert_eq!(report.kept[0].plugin, crate::keep::PluginName::FastApi);
        assert_eq!(report.kept[0].why, "registered as a route handler");
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
            dependencies: Vec::new(),
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
            dependencies: Vec::new(),
        };

        let report = analyze(&index, &Policy::none(), &manifest, &Config::default());

        assert!(report.is_clean(), "{:?}", report.findings);
    }

    #[test]
    fn documentation_example_and_script_directories_are_roots() {
        let mut index = FakeIndex::new();
        index.add_file("/proj/docs/conf.py", "docs.conf");
        index.add_file("/proj/examples/demo.py", "examples.demo");
        index.add_file("/proj/benchmarks/bench_it.py", "benchmarks.bench_it");
        index.add_file("/proj/docs_src/tutorial/one.py", "docs_src.tutorial.one");
        index.add_file("/proj/pkg/orphan.py", "pkg.orphan");

        let report = analyze_without_plugins(&index);

        let files: Vec<_> = report
            .findings
            .iter()
            .filter_map(|f| f.module.as_ref())
            .map(ModulePath::as_str)
            .collect();
        assert_eq!(files, ["pkg.orphan"]);
    }

    #[test]
    fn names_in_dunder_all_and_public_module_names_are_api() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/lib/api.py", "lib.api");
        index.add_symbol(file, "exported", SymbolKind::Function);
        index.add_symbol(file, "also_public", SymbolKind::Function);
        index.add_symbol(file, "_private", SymbolKind::Function);
        index.set_exports(file, &["exported"]);
        import_from_elsewhere(&mut index, file);

        let plain = analyze(
            &index,
            &Policy::builtin(),
            &Manifest::empty(),
            &Config::default(),
        );
        assert_eq!(symbol_names(&plain), ["also_public", "_private"]);

        let config = Config {
            public_modules: vec![crate::config::ModulePrefix::new("lib")],
            ..Config::default()
        };
        let library = analyze(&index, &Policy::builtin(), &Manifest::empty(), &config);
        assert_eq!(symbol_names(&library), ["_private"]);
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
