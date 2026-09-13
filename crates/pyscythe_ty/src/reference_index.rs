//! A single pass over every project file that resolves each name, attribute,
//! imported symbol, and dotted string to its definition and records where it
//! was used, and that records which module imports which.
//!
//! Building this once is far cheaper than searching the workspace per symbol,
//! and resolving from the use site handles aliased imports uniformly.

use pyscythe_core::index::{ImportKind, ImportedNames};
use ruff_db::files::{File, FileRange};
use ruff_db::parsed::parsed_module;
use ruff_python_ast::visitor::source_order::{SourceOrderVisitor, TraversalSignal, walk_body};
use ruff_python_ast::{self as ast, AnyNodeRef, Expr};
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::{FxHashMap, FxHashSet};
use ty_ide::document_symbols;
use ty_project::parallel::{ParallelIteratorExt, minimum_parallel_job_len};
use ty_python_semantic::{
    ImportAliasResolution, ResolvedDefinition, SemanticModel, definitions_for_attribute,
    definitions_for_imported_symbol, definitions_for_name, fixture_bindings_for_parameter,
};

use rayon::prelude::*;

const MAX_MIN_FILES_PER_JOB: usize = 32;

/// Where a definition was used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Use {
    pub(crate) file: File,
    pub(crate) range: TextRange,
}

/// The definition a use resolved to, keyed by the range of its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DefinitionKey {
    pub(crate) file: File,
    pub(crate) name_range: TextRange,
}

/// An import of something outside the project and the standard library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExternalImportRecord {
    /// First segment of the imported name.
    pub(crate) top_level: String,
    /// The statement's range.
    pub(crate) range: TextRange,
    /// The resolved file in site-packages, or `None` when unresolved.
    pub(crate) site_packages_file: Option<File>,
}

/// One file importing another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ImportEdge {
    pub(crate) target: File,
    pub(crate) range: TextRange,
    pub(crate) kind: ImportKind,
    pub(crate) names: ImportedNames,
}

/// Uses of every project definition, the import graph, and every attribute
/// name that is accessed anywhere.
#[derive(Debug, Default)]
pub(crate) struct ReferenceIndex {
    uses: FxHashMap<DefinitionKey, Vec<Use>>,
    imports: FxHashMap<File, Vec<ImportEdge>>,
    external_imports: FxHashMap<File, Vec<ExternalImportRecord>>,
    attribute_names: FxHashSet<String>,
    parameter_names: FxHashSet<String>,
}

impl ReferenceIndex {
    /// Walks every file in `files`, in parallel, recording uses of definitions
    /// that live in `files` and imports between them.
    pub(crate) fn build(db: &dyn ty_project::Db, files: &[File]) -> Self {
        let project_files: FxHashSet<File> = files.iter().copied().collect();
        let minimum_job_len = minimum_parallel_job_len(files.len(), MAX_MIN_FILES_PER_JOB);

        let per_file: Vec<(File, FileUses)> = files
            .par_iter()
            .copied()
            .with_min_len(minimum_job_len)
            .map_with_db(db, |db, file| {
                (file, collect_uses(db, file, &project_files))
            })
            .collect();

        let mut index = Self::default();
        for (file, file_uses) in per_file {
            for (key, use_site) in file_uses.uses {
                index.uses.entry(key).or_default().push(use_site);
            }
            index.imports.insert(file, file_uses.imports);
            index
                .external_imports
                .insert(file, file_uses.external_imports);
            index.attribute_names.extend(file_uses.attribute_names);
            index.parameter_names.extend(file_uses.parameter_names);
        }
        index
    }

    /// Whether `x.<name>` or `getattr(x, "<name>")` appears anywhere.
    pub(crate) fn attribute_name_is_used(&self, name: &str) -> bool {
        self.attribute_names.contains(name)
    }

    /// Adds names that non-Python sources, templates above all, refer to.
    pub(crate) fn extend_attribute_names(&mut self, names: impl IntoIterator<Item = String>) {
        self.attribute_names.extend(names);
    }

    /// Whether any function takes a parameter called `name`.
    pub(crate) fn parameter_name_is_used(&self, name: &str) -> bool {
        self.parameter_names.contains(name)
    }

    /// Every recorded use of the definition whose name occupies `key`.
    pub(crate) fn uses_of(&self, key: DefinitionKey) -> &[Use] {
        self.uses.get(&key).map_or(&[], Vec::as_slice)
    }

    /// Every import edge leaving `file`.
    pub(crate) fn imports_from(&self, file: File) -> &[ImportEdge] {
        self.imports.get(&file).map_or(&[], Vec::as_slice)
    }

    /// Every external import in `file`.
    pub(crate) fn external_imports_in(&self, file: File) -> &[ExternalImportRecord] {
        self.external_imports.get(&file).map_or(&[], Vec::as_slice)
    }
}

#[derive(Debug, Default)]
struct FileUses {
    uses: Vec<(DefinitionKey, Use)>,
    imports: Vec<ImportEdge>,
    external_imports: Vec<ExternalImportRecord>,
    attribute_names: FxHashSet<String>,
    parameter_names: FxHashSet<String>,
}

/// Builtins whose second argument names an attribute.
const REFLECTION_BUILTINS: &[&str] = &["getattr", "hasattr", "setattr", "delattr"];

fn collect_uses(db: &dyn ty_project::Db, file: File, project_files: &FxHashSet<File>) -> FileUses {
    let program_file = db.program_file(file);
    let module = parsed_module(db, program_file.python_file(db)).load(db);
    let model = SemanticModel::new(db, program_file);

    let mut collector = UseCollector {
        db,
        model: &model,
        file,
        project_files,
        function_depth: 0,
        type_checking_depth: 0,
        out: FileUses::default(),
    };
    walk_body(&mut collector, &module.syntax().body);
    collector.out
}

struct UseCollector<'a, 'db> {
    db: &'db dyn ty_project::Db,
    model: &'a SemanticModel<'db>,
    file: File,
    project_files: &'a FxHashSet<File>,
    /// How many function bodies enclose the current node.
    function_depth: u32,
    /// How many `if TYPE_CHECKING:` blocks enclose the current node.
    type_checking_depth: u32,
    out: FileUses,
}

impl UseCollector<'_, '_> {
    const fn import_kind(&self) -> ImportKind {
        if self.type_checking_depth > 0 {
            ImportKind::TypeOnly
        } else if self.function_depth > 0 {
            ImportKind::Deferred
        } else {
            ImportKind::Runtime
        }
    }

    fn record_definition_use(&mut self, use_range: TextRange, target: FileRange) {
        if !self.project_files.contains(&target.file()) {
            return;
        }
        self.out.uses.push((
            DefinitionKey {
                file: target.file(),
                name_range: target.range(),
            },
            Use {
                file: self.file,
                range: use_range,
            },
        ));
    }

    fn record_import(
        &mut self,
        range: TextRange,
        target: File,
        kind: ImportKind,
        names: ImportedNames,
    ) {
        if self.project_files.contains(&target) {
            self.out.imports.push(ImportEdge {
                target,
                range,
                kind,
                names,
            });
        }
    }

    /// Records `resolved` as uses at `use_range`; a module resolution becomes an
    /// import edge attributed to `import_range`, the enclosing statement.
    fn record(
        &mut self,
        use_range: TextRange,
        import_range: TextRange,
        resolved: Vec<ResolvedDefinition<'_>>,
    ) {
        for definition in resolved {
            match definition {
                ResolvedDefinition::Definition(_) | ResolvedDefinition::FileWithRange(_) => {
                    let target = definition.focus_range(self.db);
                    self.record_definition_use(use_range, target);
                }
                ResolvedDefinition::Module(module) => {
                    let kind = self.import_kind();
                    self.record_import(
                        import_range,
                        module.file(self.db),
                        kind,
                        ImportedNames::Explicit,
                    );
                }
            }
        }
    }

    /// `"whitenoise.middleware.WhiteNoiseMiddleware"` in `MIDDLEWARE` or
    /// `"corsheaders"` in `INSTALLED_APPS`: a distribution named in
    /// configuration is a distribution in use. Only names that resolve into
    /// the environment count; anything else is just a string.
    fn record_installed_module_use(&mut self, range: TextRange, dotted: &str) {
        let top_level = dotted.split('.').next().unwrap_or(dotted);
        if top_level.len() < 3 || !is_identifier(top_level) {
            return;
        }
        let Some(module) = self.model.resolve_module(Some(top_level), 0) else {
            return;
        };
        let installed = module
            .search_path(self.db)
            .is_some_and(|path| path.is_site_packages() || path.is_editable());
        if installed {
            self.record_external(range, top_level, 0);
        }
    }

    /// Notes an absolute import of `dotted` when it lands outside the project
    /// and the standard library, or nowhere at all.
    fn record_external(&mut self, range: TextRange, dotted: &str, level: u32) {
        if level > 0 || dotted.is_empty() {
            return;
        }
        let top_level = dotted.split('.').next().unwrap_or(dotted).to_owned();
        // `__future__` and friends are interpreter features, not distributions.
        if top_level.starts_with("__") {
            return;
        }
        let module = self.model.resolve_module(Some(&top_level), 0);
        let site_packages_file = match module {
            Some(module) => {
                let search_path = module.search_path(self.db);
                let in_environment =
                    search_path.is_some_and(|p| p.is_site_packages() || p.is_editable());
                let in_stdlib =
                    search_path.is_some_and(ty_module_resolver::SearchPath::is_standard_library);
                // typeshed bundles stubs for a few third-party distributions,
                // typing_extensions above all; those are still dependencies.
                let minor = self.model.program_file().python_version(self.db).minor;
                let really_stdlib = in_stdlib
                    && ruff_python_stdlib::sys::is_known_standard_library(minor, &top_level);
                if really_stdlib {
                    return;
                }
                if !in_environment && !in_stdlib {
                    // First-party or an extra path: not a dependency question.
                    return;
                }
                let file = module.file(self.db).filter(|_| in_environment);
                if file.is_some_and(|file| self.project_files.contains(&file)) {
                    return;
                }
                file
            }
            None => None,
        };
        self.out.external_imports.push(ExternalImportRecord {
            top_level,
            range,
            site_packages_file,
        });
    }

    /// Records an edge to `dotted` and every package above it, since importing
    /// `a.b.c` also executes `a/__init__.py` and `a/b/__init__.py`.
    /// `names` describes the import of `dotted` itself; the packages above it
    /// are only loaded on the way.
    fn record_module_and_ancestors(
        &mut self,
        range: TextRange,
        dotted: &str,
        level: u32,
        names: ImportedNames,
    ) {
        self.record_external(range, dotted, level);
        let kind = self.import_kind();
        let segments: Vec<&str> = dotted.split('.').collect();
        for end in 1..=segments.len() {
            let prefix = segments.get(..end).map(|s| s.join(".")).unwrap_or_default();
            let edge_names = if end == segments.len() {
                names
            } else {
                ImportedNames::Explicit
            };
            if let Some(module) = self.model.resolve_module(Some(prefix.as_str()), level)
                && let Some(module_file) = module.file(self.db)
            {
                self.record_import(range, module_file, kind, edge_names);
            }
        }
        if dotted.is_empty()
            && let Some(module) = self.model.resolve_module(None, level)
            && let Some(module_file) = module.file(self.db)
        {
            self.record_import(range, module_file, kind, names);
        }
    }

    /// `f"pkg.tables.{name}"` is `importlib.import_module` territory: every
    /// module under `pkg.tables` may be loaded, so each gets a deferred edge.
    fn record_dynamic_package_import(&mut self, fstring: &ast::ExprFString) {
        let Some(first) = fstring.value.iter().next() else {
            return;
        };
        let ast::FStringPart::FString(part) = first else {
            return;
        };
        let Some(ast::InterpolatedStringElement::Literal(literal)) = part.elements.iter().next()
        else {
            return;
        };
        self.record_package_wide_import(fstring.range(), &literal.value);
    }

    /// `"django.conf.locale."` followed by something computed: every module
    /// under the package may be loaded, so each gets a deferred edge.
    fn record_package_wide_import(&mut self, range: TextRange, head: &str) {
        let Some(prefix) = head.strip_suffix('.') else {
            return;
        };
        if split_dotted_reference(prefix).is_none() {
            return;
        }
        let Some(package) = self.model.resolve_module(Some(prefix), 0) else {
            return;
        };
        let Some(package_file) = package.file(self.db) else {
            return;
        };
        self.record_import(
            range,
            package_file,
            ImportKind::Deferred,
            ImportedNames::Explicit,
        );
        let submodule_files: Vec<File> = package
            .all_submodules(self.db)
            .iter()
            .filter_map(|module| module.file(self.db))
            .collect();
        for file in submodule_files {
            self.record_import(range, file, ImportKind::Deferred, ImportedNames::Explicit);
        }
    }

    /// A string such as `"pkg.settings.DEBUG"` or `"pkg.cli:main"` counts as a
    /// use of that symbol and a deferred import of its module.
    fn record_string_reference(&mut self, literal: &ast::ExprStringLiteral) {
        let text = literal.value.to_str();
        // `"django.conf.locale.%s"`, `"{}.formats"`: a module path with a hole
        // for `%` or `str.format` to fill is a dynamic import of the package.
        if let Some(head) = formatted_module_head(text) {
            self.record_package_wide_import(literal.range(), head);
        }
        // Frameworks name attributes in strings: Django's `list_display`,
        // DRF's `fields`, `getattr` lookups. Any identifier-shaped literal keeps
        // a same-named method alive.
        if is_identifier(text) {
            self.out.attribute_names.insert(text.to_owned());
            self.record_installed_module_use(literal.range(), text);
        }
        let Some((module_name, attribute)) = split_dotted_reference(text) else {
            return;
        };
        self.record_installed_module_use(literal.range(), module_name);
        let Some(module) = self.model.resolve_module(Some(module_name), 0) else {
            return;
        };
        let Some(module_file) = module.file(self.db) else {
            return;
        };
        self.record_import(
            literal.range(),
            module_file,
            ImportKind::Deferred,
            ImportedNames::Explicit,
        );

        let Some(attribute) = attribute else {
            return;
        };
        if !self.project_files.contains(&module_file) {
            return;
        }
        let program_file = self.db.program_file(module_file);
        let symbols = document_symbols(self.db, program_file);
        let matches: Vec<TextRange> = symbols
            .iter()
            .filter(|(_, info)| info.name == attribute)
            .map(|(_, info)| info.name_range)
            .collect();
        for name_range in matches {
            self.record_definition_use(literal.range(), FileRange::new(module_file, name_range));
        }
    }
}

/// The literal head of a module path template, up to and including the dot
/// before the first `%s`, `%(name)s`, or `{}` placeholder: `"pkg.locale."`
/// for `"pkg.locale.%s"`. `None` when the text has no placeholder or no
/// dotted head before it.
fn formatted_module_head(text: &str) -> Option<&str> {
    let hole = text.find(['%', '{'])?;
    let head = text.get(..hole)?;
    if !head.ends_with('.') || head.contains(char::is_whitespace) {
        return None;
    }
    let dotted = head.strip_suffix('.')?;
    let mut segments = dotted.split('.');
    segments.all(is_identifier).then_some(head)
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Splits `"a.b.c"` into `("a.b", Some("c"))` and `"a.b:c"` into the same,
/// returning `None` for text that is not a plausible dotted reference.
fn split_dotted_reference(text: &str) -> Option<(&str, Option<&str>)> {
    if text.len() > 200 || text.contains(char::is_whitespace) {
        return None;
    }
    let (module, attribute) = match text.split_once(':') {
        Some((module, attribute)) => (module, Some(attribute)),
        None => match text.rsplit_once('.') {
            Some((module, attribute)) if module.contains('.') => (module, Some(attribute)),
            _ => (text, None),
        },
    };
    let is_identifier = |s: &str| {
        let mut chars = s.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    let segments: Vec<&str> = module.split('.').collect();
    if segments.len() < 2 || !segments.iter().all(|s| is_identifier(s)) {
        return None;
    }
    if attribute.is_some_and(|a| !is_identifier(a)) {
        return None;
    }
    Some((module, attribute))
}

fn is_type_checking_test(test: &Expr) -> bool {
    match test {
        Expr::Name(name) => name.id.as_str() == "TYPE_CHECKING",
        Expr::Attribute(attribute) => attribute.attr.as_str() == "TYPE_CHECKING",
        _ => false,
    }
}

impl<'a> SourceOrderVisitor<'a> for UseCollector<'a, '_> {
    fn enter_node(&mut self, node: AnyNodeRef<'a>) -> TraversalSignal {
        match node {
            AnyNodeRef::StmtFunctionDef(_) => self.function_depth += 1,
            AnyNodeRef::StmtIf(if_statement) if is_type_checking_test(&if_statement.test) => {
                self.type_checking_depth += 1;
            }
            AnyNodeRef::ExprName(name) if name.ctx.is_load() => {
                let resolved = definitions_for_name(
                    self.model,
                    name.id.as_str(),
                    node,
                    ImportAliasResolution::ResolveAliases,
                );
                self.record(name.range(), name.range(), resolved);
            }
            AnyNodeRef::ExprAttribute(attribute) if attribute.ctx.is_load() => {
                self.out
                    .attribute_names
                    .insert(attribute.attr.as_str().to_owned());
                let resolved = definitions_for_attribute(self.model, attribute);
                self.record(attribute.attr.range(), attribute.range(), resolved);
            }
            AnyNodeRef::ExprCall(call) => {
                if let Expr::Name(callee) = &*call.func
                    && REFLECTION_BUILTINS.contains(&callee.id.as_str())
                    && let Some(Expr::StringLiteral(name)) = call.arguments.args.get(1)
                {
                    self.out
                        .attribute_names
                        .insert(name.value.to_str().to_owned());
                }
            }
            AnyNodeRef::ExprStringLiteral(literal) => {
                self.record_string_reference(literal);
            }
            AnyNodeRef::ExprFString(fstring) => {
                self.record_dynamic_package_import(fstring);
            }
            AnyNodeRef::Parameter(parameter) => {
                self.out
                    .parameter_names
                    .insert(parameter.name.as_str().to_owned());
                // A test parameter names a pytest fixture; ty resolves which one.
                let index = ty_python_core::semantic_index(self.db, self.model.program_file());
                let definition = index.expect_single_definition(parameter);
                let fixtures: Vec<ResolvedDefinition<'_>> =
                    fixture_bindings_for_parameter(self.db, definition)
                        .iter()
                        .map(|binding| ResolvedDefinition::Definition(binding.fixture()))
                        .collect();
                self.record(parameter.range(), parameter.range(), fixtures);
            }
            AnyNodeRef::StmtImportFrom(import) => {
                let module_name = import.module.as_deref().unwrap_or_default();
                let names = if import.names.iter().any(|alias| alias.name.as_str() == "*") {
                    ImportedNames::Wildcard
                } else {
                    ImportedNames::Explicit
                };
                self.record_module_and_ancestors(import.range(), module_name, import.level, names);
                for alias in &import.names {
                    let name = alias.name.as_str();
                    if name == "*" {
                        continue;
                    }
                    let resolved = definitions_for_imported_symbol(
                        self.model,
                        import,
                        name,
                        ImportAliasResolution::ResolveAliases,
                    );
                    self.record(alias.name.range(), import.range(), resolved);
                }
            }
            AnyNodeRef::StmtImport(import) => {
                for alias in &import.names {
                    self.record_module_and_ancestors(
                        alias.range(),
                        alias.name.id.as_str(),
                        0,
                        ImportedNames::Explicit,
                    );
                }
            }
            _ => {}
        }
        TraversalSignal::Traverse
    }

    fn leave_node(&mut self, node: AnyNodeRef<'a>) {
        match node {
            AnyNodeRef::StmtFunctionDef(_) => self.function_depth -= 1,
            AnyNodeRef::StmtIf(if_statement) if is_type_checking_test(&if_statement.test) => {
                self.type_checking_depth -= 1;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn formatted_module_heads_are_dotted_paths_before_a_placeholder() {
        use super::formatted_module_head;
        assert_eq!(
            formatted_module_head("django.conf.locale.%s"),
            Some("django.conf.locale.")
        );
        assert_eq!(formatted_module_head("pkg.tables.{}"), Some("pkg.tables."));
        assert_eq!(formatted_module_head("%s.formats"), None);
        assert_eq!(formatted_module_head("pkg.tables"), None);
        assert_eq!(formatted_module_head("Total: %s items."), None);
    }

    use super::split_dotted_reference;

    #[test]
    fn splits_dotted_and_colon_references() {
        assert_eq!(
            split_dotted_reference("pkg.settings.DEBUG"),
            Some(("pkg.settings", Some("DEBUG")))
        );
        assert_eq!(
            split_dotted_reference("pkg.cli:main"),
            Some(("pkg.cli", Some("main")))
        );
        assert_eq!(
            split_dotted_reference("pkg.tasks"),
            Some(("pkg.tasks", None))
        );
    }

    #[test]
    fn rejects_text_that_is_not_a_reference() {
        assert_eq!(split_dotted_reference("hello world"), None);
        assert_eq!(split_dotted_reference("plain"), None);
        assert_eq!(split_dotted_reference("1.5"), None);
        assert_eq!(split_dotted_reference("a.b-c"), None);
        // `file.txt` is syntactically a module path; resolution rejects it later.
    }
}
