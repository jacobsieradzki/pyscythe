//! A single pass over every project file that resolves each name, attribute,
//! imported symbol, and dotted string to its definition and records where it
//! was used, and that records which module imports which.
//!
//! Building this once is far cheaper than searching the workspace per symbol,
//! and resolving from the use site handles aliased imports uniformly.

use pyscythe_core::index::{ImportCondition, ImportKind, ImportedNames};
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
use salsa::{Cancelled, Database as _};
use std::panic::AssertUnwindSafe;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use ty_project::ProjectDatabase;

const MAX_MIN_FILES_PER_JOB: usize = 32;
/// How often the watchdog looks for a file over its budget.
const WATCHDOG_POLL: Duration = Duration::from_millis(50);

/// How long ty may spend on one file's types before the file falls back to matching by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileBudget(pub Duration);

/// How the references in a file are found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resolution {
    /// Through ty: a use points at the definition it resolves to.
    Semantic,
    /// By name only, for a file whose types ty cannot infer in reasonable time.
    ByName,
}

/// The files the analysis reports on, and where a stub's implementation lives.
///
/// A module that ships an inline `.pyi` stub resolves to the stub, which is
/// not analysed; the import still means the implementation beside it.
#[derive(Debug, Default)]
pub(crate) struct ProjectFiles {
    files: FxHashSet<File>,
    /// The implementation a stub describes, and where each of its names is
    /// defined, keyed by the stub file.
    stubbed: FxHashMap<File, Implementation>,
}

/// Where the names a stub declares are really defined.
#[derive(Debug)]
struct Implementation {
    file: File,
    /// The name each definition in the stub carries.
    names_in_stub: FxHashMap<TextRange, String>,
    /// Where that name is defined in the implementation.
    ranges_by_name: FxHashMap<String, TextRange>,
}

impl ProjectFiles {
    fn new(db: &dyn ty_project::Db, files: &[File]) -> Self {
        let by_path: FxHashMap<String, File> = files
            .iter()
            .map(|file| (file.path(db).to_string(), *file))
            .collect();
        let mut stubbed = FxHashMap::default();
        // Sorted so the table is built the same way on every run.
        let mut paths: Vec<(&String, &File)> = by_path.iter().collect();
        paths.sort_by_key(|(path, _)| *path);
        for (path, stub) in paths {
            let Some(stem) = path.strip_suffix(".pyi") else {
                continue;
            };
            let Some(implementation) = by_path.get(&format!("{stem}.py")) else {
                continue;
            };
            stubbed.insert(
                *stub,
                Implementation {
                    file: *implementation,
                    names_in_stub: definitions_in(db, *stub)
                        .into_iter()
                        .map(|(name, range)| (range, name))
                        .collect(),
                    ranges_by_name: definitions_in(db, *implementation).into_iter().collect(),
                },
            );
        }
        Self {
            files: files.iter().copied().collect(),
            stubbed,
        }
    }

    fn contains(&self, file: File) -> bool {
        self.files.contains(&file)
    }

    /// The file an import of `file` really reaches: the implementation when
    /// `file` is a stub that describes one, and `file` itself otherwise.
    fn resolve(&self, file: File) -> File {
        self.stubbed
            .get(&file)
            .map_or(file, |implementation| implementation.file)
    }

    /// The definition an import really reaches. A name declared in a stub is
    /// defined in the implementation beside it, under the same name.
    fn resolve_definition(&self, file: File, range: TextRange) -> (File, TextRange) {
        let Some(implementation) = self.stubbed.get(&file) else {
            return (file, range);
        };
        let moved = implementation
            .names_in_stub
            .get(&range)
            .and_then(|name| implementation.ranges_by_name.get(name))
            .copied();
        (implementation.file, moved.unwrap_or(range))
    }
}

/// Every definition in the file, by name and where the name is written.
fn definitions_in(db: &dyn ty_project::Db, file: File) -> Vec<(String, TextRange)> {
    document_symbols(db, db.program_file(file))
        .iter()
        .map(|(_, info)| (info.name.to_string(), info.name_range))
        .collect()
}

/// Which files still need walking, and how each one is to be walked.
///
/// A file ty cannot type within the budget is walked again with its names
/// matched textually; one that is still over budget then is given up on, so
/// the schedule always empties.
#[derive(Debug)]
struct Schedule<T> {
    pending: Vec<T>,
    by_name: FxHashSet<T>,
    abandoned: FxHashSet<T>,
}

impl<T: Copy + Eq + std::hash::Hash> Schedule<T> {
    fn new(files: &[T]) -> Self {
        Self {
            pending: files.to_vec(),
            by_name: FxHashSet::default(),
            abandoned: FxHashSet::default(),
        }
    }

    const fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// The files to walk next; the schedule is left empty for them to be put back into.
    fn take_pending(&mut self) -> Vec<T> {
        std::mem::take(&mut self.pending)
    }

    fn resolution_of(&self, file: T) -> Resolution {
        if self.by_name.contains(&file) {
            Resolution::ByName
        } else {
            Resolution::Semantic
        }
    }

    /// A walk cut short by another file's cancellation: walk it again, unchanged.
    fn retry(&mut self, file: T) {
        if !self.abandoned.contains(&file) {
            self.pending.push(file);
        }
    }

    /// A walk that ran past the budget: demote it, or give up if it is already by name.
    fn over_budget(&mut self, file: T) {
        if self.by_name.insert(file) {
            self.pending.push(file);
        } else {
            self.by_name.remove(&file);
            self.abandoned.insert(file);
            self.pending.retain(|pending| pending != &file);
        }
    }
}

/// One walk over some of the files.
#[derive(Debug, Default)]
struct Pass {
    completed: Vec<(File, FileUses, Duration)>,
    /// Files whose walk was cut short by a cancellation; they run again.
    cancelled: Vec<File>,
    /// Files that caused the cancellation; they run again by name.
    over_budget: Vec<File>,
}

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
    /// Whether every interpreter runs the import.
    pub(crate) condition: ImportCondition,
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
    attribute_prefixes: FxHashSet<String>,
    parameter_names: FxHashSet<String>,
    script_paths: FxHashSet<String>,
    /// Files that build something out of their own module namespace.
    globals_readers: FxHashSet<File>,
    /// Files whose references were matched by name because ty ran over budget on them.
    by_name: Vec<File>,
    /// Files over budget even by name, so nothing in them counts as a reference.
    abandoned: Vec<File>,
    /// The file ty spent longest on, for tuning the budget.
    slowest: Option<(File, Duration)>,
}

impl ReferenceIndex {
    /// Walks every file in `files`, in parallel, recording uses of definitions
    /// that live in `files` and imports between them.
    ///
    /// A file that keeps ty busy beyond `budget` is cut off, through salsa's
    /// cancellation, and walked again with its names matched textually, so
    /// one pathological file cannot stall the whole run. `db` must be the only
    /// live handle on the database: cancellation waits for every other one.
    pub(crate) fn build(db: &mut ProjectDatabase, files: &[File], budget: FileBudget) -> Self {
        let project_files = ProjectFiles::new(db, files);
        let mut schedule = Schedule::new(files);
        let mut index = Self::default();
        while !schedule.is_empty() {
            let walking = schedule.take_pending();
            let pass = budgeted_pass(db, &walking, &project_files, &schedule, budget);
            index.absorb(pass.completed);
            for file in pass.over_budget {
                schedule.over_budget(file);
            }
            for file in pass.cancelled {
                schedule.retry(file);
            }
        }
        index.by_name = schedule.by_name.into_iter().collect();
        index.abandoned = schedule.abandoned.into_iter().collect();
        index
    }

    /// [`Self::build`] without a budget, for callers that cannot hand over the database.
    pub(crate) fn build_unbudgeted(db: &dyn ty_project::Db, files: &[File]) -> Self {
        let project_files = ProjectFiles::new(db, files);
        let pass = run_pass(
            db,
            files,
            &project_files,
            &Schedule::new(&[]),
            &Mutex::default(),
        );
        let mut index = Self::default();
        index.absorb(pass.completed);
        index
    }

    /// Files whose references were matched by name only.
    pub(crate) fn files_by_name(&self) -> &[File] {
        &self.by_name
    }

    /// Files left out of the index: over budget even when matched by name.
    pub(crate) fn files_abandoned(&self) -> &[File] {
        &self.abandoned
    }

    /// The file ty spent longest on, and how long.
    pub(crate) const fn slowest_file(&self) -> Option<(File, Duration)> {
        self.slowest
    }

    fn absorb(&mut self, completed: Vec<(File, FileUses, Duration)>) {
        let index = self;
        for (file, file_uses, elapsed) in completed {
            if index.slowest.is_none_or(|(_, slowest)| elapsed > slowest) {
                index.slowest = Some((file, elapsed));
            }
            for (key, use_site) in file_uses.uses {
                index.uses.entry(key).or_default().push(use_site);
            }
            index.imports.insert(file, file_uses.imports);
            index
                .external_imports
                .insert(file, file_uses.external_imports);
            index.attribute_names.extend(file_uses.attribute_names);
            index
                .attribute_prefixes
                .extend(file_uses.attribute_prefixes);
            index.parameter_names.extend(file_uses.parameter_names);
            index.script_paths.extend(file_uses.script_paths);
            if file_uses.reads_own_globals {
                index.globals_readers.insert(file);
            }
        }
    }

    /// Whether `x.<name>` or `getattr(x, "<name>")` appears anywhere, or a
    /// name is built from a prefix of it (`getattr(self, f"visit_{kind}")`).
    pub(crate) fn attribute_name_is_used(&self, name: &str) -> bool {
        self.attribute_names.contains(name)
            || self
                .attribute_prefixes
                .iter()
                .any(|prefix| name.starts_with(prefix.as_str()))
    }

    /// Whether the file reads the namespace it defines.
    pub(crate) fn reads_own_globals(&self, file: File) -> bool {
        self.globals_readers.contains(&file)
    }

    /// Every string literal that names a `.py` file.
    pub(crate) fn script_paths(&self) -> impl Iterator<Item = &str> {
        self.script_paths.iter().map(String::as_str)
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
    attribute_prefixes: FxHashSet<String>,
    parameter_names: FxHashSet<String>,
    script_paths: FxHashSet<String>,
    /// Whether the file enumerates its own namespace with `globals()`.
    reads_own_globals: bool,
}

/// Builtins whose second argument names an attribute.
const REFLECTION_BUILTINS: &[&str] = &["getattr", "hasattr", "setattr", "delattr"];

/// Calls that hand back the module's own namespace.
const NAMESPACE_BUILTINS: &[&str] = &["globals", "vars"];

/// Walks `files` in parallel, with the watchdog on this thread: when a file
/// runs past its budget, every other database handle is told to stop and the
/// pass ends early with that file marked over budget.
fn budgeted_pass(
    db: &mut ProjectDatabase,
    files: &[File],
    project_files: &ProjectFiles,
    schedule: &Schedule<File>,
    budget: FileBudget,
) -> Pass {
    let in_flight: Mutex<FxHashMap<File, Instant>> = Mutex::default();
    let worker = ty_project::Db::dyn_clone(db);
    let mut over_budget: Vec<File> = Vec::new();
    let mut pass = std::thread::scope(|scope| {
        let walk = scope.spawn(|| {
            let pass = run_pass(&*worker, files, project_files, schedule, &in_flight);
            drop(worker);
            pass
        });
        while !walk.is_finished() {
            std::thread::sleep(WATCHDOG_POLL);
            let late: Vec<File> = in_flight
                .lock()
                .map(|flight| {
                    flight
                        .iter()
                        .filter(|(_, started)| started.elapsed() > budget.0)
                        .map(|(file, _)| *file)
                        .collect()
                })
                .unwrap_or_default();
            if !late.is_empty() {
                over_budget = late;
                // Salsa cancels every other handle before any write; the write
                // itself is nothing, since the LRU is off.
                db.trigger_lru_eviction();
                break;
            }
        }
        walk.join()
            .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
    });
    pass.cancelled.retain(|file| !over_budget.contains(file));
    pass.over_budget = over_budget;
    pass
}

fn run_pass(
    db: &dyn ty_project::Db,
    files: &[File],
    project_files: &ProjectFiles,
    schedule: &Schedule<File>,
    in_flight: &Mutex<FxHashMap<File, Instant>>,
) -> Pass {
    let minimum_job_len = minimum_parallel_job_len(files.len(), MAX_MIN_FILES_PER_JOB);
    let outcomes: Vec<(File, Option<(FileUses, Duration)>)> = files
        .par_iter()
        .copied()
        .with_min_len(minimum_job_len)
        .map_with_db(db, |db, file| {
            let resolution = schedule.resolution_of(file);
            let started = Instant::now();
            if let Ok(mut flight) = in_flight.lock() {
                flight.insert(file, started);
            }
            let outcome = Cancelled::catch(AssertUnwindSafe(|| {
                collect_uses(db, file, project_files, resolution)
            }));
            if let Ok(mut flight) = in_flight.lock() {
                flight.remove(&file);
            }
            (file, outcome.ok().map(|uses| (uses, started.elapsed())))
        })
        .collect();

    let mut pass = Pass::default();
    for (file, outcome) in outcomes {
        match outcome {
            Some((uses, elapsed)) => pass.completed.push((file, uses, elapsed)),
            None => pass.cancelled.push(file),
        }
    }
    pass
}

fn collect_uses(
    db: &dyn ty_project::Db,
    file: File,
    project_files: &ProjectFiles,
    resolution: Resolution,
) -> FileUses {
    let program_file = db.program_file(file);
    let module = parsed_module(db, program_file.python_file(db)).load(db);
    let model = SemanticModel::new(db, program_file);

    let mut collector = UseCollector {
        db,
        model: &model,
        file,
        project_files,
        resolution,
        function_depth: 0,
        type_checking_depth: 0,
        version_guard_depth: 0,
        version_flags: FxHashSet::default(),
        out: FileUses::default(),
    };
    walk_body(&mut collector, &module.syntax().body);
    collector.out
}

struct UseCollector<'a, 'db> {
    db: &'db dyn ty_project::Db,
    model: &'a SemanticModel<'db>,
    file: File,
    project_files: &'a ProjectFiles,
    resolution: Resolution,
    /// How many function bodies enclose the current node.
    function_depth: u32,
    /// How many `if TYPE_CHECKING:` blocks enclose the current node.
    type_checking_depth: u32,
    /// How many `if sys.version_info ...:` statements enclose the current node.
    version_guard_depth: u32,
    /// Names assigned from a version test, as in `PY_3_14_PLUS = sys.version_info >= (3, 14)`.
    version_flags: FxHashSet<String>,
    out: FileUses,
}

impl UseCollector<'_, '_> {
    /// Whether the test decides something from the Python version, following
    /// a name the file assigned from such a test.
    fn tests_the_version(&self, test: &Expr) -> bool {
        if let Expr::Name(name) = test
            && self.version_flags.contains(name.id.as_str())
        {
            return true;
        }
        is_version_test(test)
    }

    /// Records `PY_3_14_PLUS = sys.version_info >= (3, 14)` so that a later
    /// `if PY_3_14_PLUS:` reads as the version test it stands for.
    fn note_version_flag(&mut self, targets: &[Expr], value: &Expr) {
        if !is_version_test(value) {
            return;
        }
        for target in targets {
            if let Expr::Name(name) = target {
                self.version_flags.insert(name.id.as_str().to_owned());
            }
        }
    }

    /// Records `from x import a, b`: the module itself and each name it binds.
    fn record_import_from(&mut self, import: &ast::StmtImportFrom) {
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
            if self.resolution == Resolution::ByName {
                self.out.attribute_names.insert(name.to_owned());
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

    /// Whether the import the collector is looking at runs on every interpreter.
    const fn import_condition(&self) -> ImportCondition {
        if self.version_guard_depth > 0 {
            ImportCondition::InterpreterVersion
        } else {
            ImportCondition::Always
        }
    }

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
        let (file, name_range) = self
            .project_files
            .resolve_definition(target.file(), target.range());
        if !self.project_files.contains(file) {
            return;
        }
        self.out.uses.push((
            DefinitionKey { file, name_range },
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
        let target = self.project_files.resolve(target);
        if self.project_files.contains(target) {
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
                if file.is_some_and(|file| self.project_files.contains(file)) {
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
            condition: self.import_condition(),
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
        if let Some(prefix) = attribute_prefix(&literal.value) {
            self.out.attribute_prefixes.insert(prefix.to_owned());
        }
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
        // `"poetry.console.commands." + name`: the same, by concatenation.
        if text.ends_with('.') {
            self.record_package_wide_import(literal.range(), text);
        }
        // `"_dt_" + name`, `"_get_current_%s" % kind`: a name built from a
        // prefix reaches every attribute that starts with it.
        let head = text.split(['%', '{']).next().unwrap_or(text);
        if let Some(prefix) = attribute_prefix(head) {
            self.out.attribute_prefixes.insert(prefix.to_owned());
        }
        // `"plugin_success.py"`, `"scripts/migrate.py"`: a script run by path.
        let is_script = std::path::Path::new(text)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("py"));
        if is_script && text.len() <= 200 && !text.contains(char::is_whitespace) {
            self.out.script_paths.insert(text.replace('\\', "/"));
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
        if !self.project_files.contains(module_file) {
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

/// `visit_`, `_dt_`, `_get_current_`: an identifier-shaped head ending in an
/// underscore, long enough to mean something, that a name is built from.
fn attribute_prefix(head: &str) -> Option<&str> {
    (head.len() >= 3 && head.ends_with('_') && is_identifier(head)).then_some(head)
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

/// Whether the test decides something from the running Python version, as in
/// `sys.version_info >= (3, 14)` or `sys.version_info[:2] < (3, 11)`.
///
/// Every branch of such a statement, `else` included, belongs to some
/// interpreter but not to this one.
fn is_version_test(test: &Expr) -> bool {
    match test {
        Expr::Name(name) => name.id.as_str() == "version_info",
        Expr::Attribute(attribute) => {
            attribute.attr.as_str() == "version_info" || is_version_test(&attribute.value)
        }
        Expr::Subscript(subscript) => is_version_test(&subscript.value),
        Expr::Compare(compare) => {
            is_version_test(&compare.left) || compare.comparators.iter().any(is_version_test)
        }
        Expr::BoolOp(operation) => operation.values.iter().any(is_version_test),
        Expr::UnaryOp(operation) => is_version_test(&operation.operand),
        Expr::Call(call) => is_version_test(&call.func),
        _ => false,
    }
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
            AnyNodeRef::StmtIf(if_statement) if self.tests_the_version(&if_statement.test) => {
                self.version_guard_depth += 1;
            }
            AnyNodeRef::StmtAssign(assign) => {
                self.note_version_flag(&assign.targets, &assign.value);
            }
            AnyNodeRef::StmtAnnAssign(assign) => {
                if let Some(value) = &assign.value {
                    self.note_version_flag(std::slice::from_ref(&assign.target), value);
                }
            }
            AnyNodeRef::ExprName(name) if name.ctx.is_load() => match self.resolution {
                Resolution::Semantic => {
                    let resolved = definitions_for_name(
                        self.model,
                        name.id.as_str(),
                        node,
                        ImportAliasResolution::ResolveAliases,
                    );
                    self.record(name.range(), name.range(), resolved);
                }
                Resolution::ByName => {
                    self.out.attribute_names.insert(name.id.as_str().to_owned());
                }
            },
            AnyNodeRef::ExprAttribute(attribute) if attribute.ctx.is_load() => {
                self.out
                    .attribute_names
                    .insert(attribute.attr.as_str().to_owned());
                if self.resolution == Resolution::Semantic {
                    let resolved = definitions_for_attribute(self.model, attribute);
                    self.record(attribute.attr.range(), attribute.range(), resolved);
                }
            }
            AnyNodeRef::ExprCall(call) => {
                if let Expr::Name(callee) = &*call.func
                    && NAMESPACE_BUILTINS.contains(&callee.id.as_str())
                    && call.arguments.args.is_empty()
                {
                    self.out.reads_own_globals = true;
                }
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
                if self.resolution == Resolution::ByName {
                    return TraversalSignal::Traverse;
                }
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
            AnyNodeRef::StmtImportFrom(import) => self.record_import_from(import),
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
            AnyNodeRef::StmtIf(if_statement) if self.tests_the_version(&if_statement.test) => {
                self.version_guard_depth -= 1;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Resolution, Schedule};

    #[test]
    fn a_file_over_budget_is_walked_by_name_and_then_given_up_on() {
        let mut schedule = Schedule::new(&[1_u32, 2]);
        assert_eq!(schedule.take_pending(), vec![1, 2]);
        assert_eq!(schedule.resolution_of(1), Resolution::Semantic);

        schedule.over_budget(1);
        assert_eq!(schedule.resolution_of(1), Resolution::ByName);
        assert_eq!(schedule.take_pending(), vec![1], "walked again, by name");

        schedule.over_budget(1);
        assert!(schedule.abandoned.contains(&1), "given up on, not retried");
        assert!(schedule.is_empty());
        assert!(
            !schedule.by_name.contains(&1),
            "abandoned, not matched by name"
        );
    }

    #[test]
    fn a_cancelled_walk_runs_again_unless_the_file_was_given_up_on() {
        let mut schedule = Schedule::new(&[1_u32]);
        let _ = schedule.take_pending();
        schedule.retry(1);
        assert_eq!(schedule.take_pending(), vec![1]);

        schedule.over_budget(1);
        let _ = schedule.take_pending();
        schedule.over_budget(1);
        schedule.retry(1);
        assert!(
            schedule.is_empty(),
            "an abandoned file is never scheduled again"
        );
    }

    #[test]
    fn attribute_prefixes_are_identifier_heads_ending_in_an_underscore() {
        use super::attribute_prefix;
        assert_eq!(attribute_prefix("visit_"), Some("visit_"));
        assert_eq!(attribute_prefix("_dt_"), Some("_dt_"));
        assert_eq!(attribute_prefix("x_"), None, "too short to mean anything");
        assert_eq!(attribute_prefix("visit"), None);
        assert_eq!(attribute_prefix("a.b_"), None);
    }

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
