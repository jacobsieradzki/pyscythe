//! Implements [`CodebaseIndex`] using the ty project database.
//!
//! ty discovers the project, resolves the Python environment, parses every
//! file, and answers semantic questions such as "where is this name used?".
//! This crate only translates between ty's types and pyscythe's domain.

use std::sync::OnceLock;

use camino::{Utf8Path, Utf8PathBuf};
use pyscythe_core::config::{NotebookPolicy, PathPatterns};
use pyscythe_core::edit::Deletable;
use pyscythe_core::index::{
    Ancestry, CodebaseIndex, ExternalImport, Import, ImportOrigin, Inheritance, NameUsage,
    Reference, SubclassRegistration, Suppression,
};
use pyscythe_core::metrics::FunctionMetrics;
use pyscythe_core::source::{
    ByteOffset, ByteSpan, Column, FileId, Line, MainGuard, ModulePath, Position, SourceFile,
};
use pyscythe_core::symbol::{DottedName, Symbol, SymbolId, SymbolKind, SymbolName, SymbolScope};
use pyscythe_core::tokens::{CloneMode, CloneToken};
use ruff_db::files::File;
use ruff_db::parsed::parsed_module;
use ruff_db::source::{line_index, source_text};
use ruff_db::system::{OsSystem, System as _, SystemPath};
use ruff_python_ast::token::TokenKind;
use ruff_text_size::{Ranged, TextRange, TextSize};
use rustc_hash::FxHashMap;
use ty_ide::{HierarchicalSymbols, SymbolInfo, document_symbols};
use ty_project::metadata::ProjectMetadataError;
use ty_project::{Db as _, ProjectDatabase, ProjectMetadata};
use ty_python_semantic::Db as _;

use crate::reference_index::{DefinitionKey, ReferenceIndex};
use pyscythe_core::manifest::DistributionName;

mod declarations;
mod distributions;
mod inheritance;
mod reference_index;
mod templates;

/// Why a project could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// The path is not valid UTF-8, which ty requires.
    #[error("project path is not valid UTF-8: {0}")]
    NonUtf8Path(std::path::PathBuf),
    /// ty could not find or parse the project metadata.
    #[error("could not discover project: {0}")]
    Discovery(#[from] ProjectMetadataError),
    /// ty rejected the project or environment configuration.
    #[error("could not configure project: {0}")]
    Configuration(#[source] anyhow::Error),
}

/// Which files to report on.
#[derive(Debug, Clone)]
pub struct IndexOptions {
    /// Project-relative globs for files to leave out of reports.
    pub exclude: PathPatterns,
    /// Whether notebooks are analysed at all.
    pub notebooks: NotebookPolicy,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            exclude: PathPatterns::none(),
            notebooks: NotebookPolicy::Exclude,
        }
    }
}

/// A [`CodebaseIndex`] backed by ty.
///
/// Excluded files are still walked for references and imports, so a use from
/// an excluded script keeps a symbol alive; they are just never reported on.
pub struct TyIndex {
    db: ProjectDatabase,
    /// Reported files, indexed by [`FileId`].
    files: Vec<File>,
    /// Every analysed file, including excluded ones.
    all_files: Vec<File>,
    ids: FxHashMap<File, FileId>,
    sources: Vec<SourceFile>,
    references: OnceLock<ReferenceIndex>,
    distributions: std::sync::Mutex<distributions::DistributionIndex>,
}

impl std::fmt::Debug for TyIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TyIndex")
            .field("files", &self.sources.len())
            .finish_non_exhaustive()
    }
}

impl TyIndex {
    /// Opens the project rooted at (or containing) `path`.
    ///
    /// # Errors
    ///
    /// Returns [`OpenError`] when the path is not UTF-8, ty cannot discover a
    /// project, or the configuration is invalid.
    pub fn open(path: &std::path::Path) -> Result<Self, OpenError> {
        // ty asserts that its working directory is absolute, so resolve relative paths first.
        let absolute = std::path::absolute(path).map_err(|error| {
            OpenError::Configuration(anyhow::anyhow!(
                "cannot resolve {}: {error}",
                path.display()
            ))
        })?;
        let root = Utf8Path::from_path(&absolute)
            .ok_or_else(|| OpenError::NonUtf8Path(absolute.clone()))?;
        let root = SystemPath::new(root);
        let system = OsSystem::new(root);

        // The directory asked for is the project when it carries a manifest
        // of its own, even a setup.py or requirements.txt; otherwise the
        // closest pyproject.toml above it is. ty's default discovery would
        // ask uv for the workspace root and prefer that, which lets a
        // monorepo root (or this repository, packaged for PyPI) swallow the
        // project actually asked for; nested manifests are handled here.
        let has_own_manifest = ["setup.py", "setup.cfg", "requirements.txt"]
            .iter()
            .any(|name| system.is_file(&root.join(name)))
            && !system.is_file(&root.join("pyproject.toml"));
        let mut metadata = if has_own_manifest {
            ProjectMetadata::new(root.file_name().unwrap_or("root"), root.to_path_buf())
        } else {
            ProjectMetadata::discover_without_uv(root, &system)?
        };
        metadata
            .apply_configuration_files(&system)
            .map_err(|error| OpenError::Configuration(error.into()))?;

        let mut db =
            ProjectDatabase::fallible(metadata, system).map_err(OpenError::Configuration)?;
        ruff_db::disable_lru(&mut db);
        db.freeze_open_files();
        db.freeze();

        let mut index = Self {
            db,
            files: Vec::new(),
            all_files: Vec::new(),
            ids: FxHashMap::default(),
            sources: Vec::new(),
            references: OnceLock::new(),
            distributions: std::sync::Mutex::new(distributions::DistributionIndex::default()),
        };
        index.select_files(&IndexOptions::default())?;
        Ok(index)
    }

    /// Chooses which files are analysed and which are reported on.
    ///
    /// Call this after reading the project's settings; it discards any
    /// reference index built so far.
    ///
    /// # Errors
    ///
    /// Returns [`OpenError`] when the project has more files than can be numbered.
    pub fn select_files(&mut self, options: &IndexOptions) -> Result<(), OpenError> {
        let db = &self.db;
        // Notebooks are scratch space; nobody deletes cells because a linter said so.
        let mut all_files: Vec<File> = db
            .project()
            .files(db)
            .iter()
            .filter(|file| {
                options.notebooks == NotebookPolicy::Include
                    || !is_notebook(file.path(db).to_string().as_str())
            })
            .collect();
        all_files.sort_by_key(|file| file.path(db).to_string());

        let project_root = db.project().root(db).as_utf8_path().to_path_buf();
        let files: Vec<File> = all_files
            .iter()
            .copied()
            .filter(|file| {
                let path = Utf8PathBuf::from(file.path(db).to_string());
                // Stubs declare, they do not define: nothing in them is dead or alive.
                if path.extension() == Some("pyi") {
                    return false;
                }
                let relative = path.strip_prefix(&project_root).unwrap_or(&path);
                !options.exclude.matches(relative)
            })
            .collect();

        let mut ids = FxHashMap::default();
        let mut sources = Vec::with_capacity(files.len());
        for (index, file) in files.iter().enumerate() {
            let id = FileId::new(u32::try_from(index).map_err(|error| {
                OpenError::Configuration(anyhow::anyhow!("too many files: {error}"))
            })?);
            ids.insert(*file, id);
            let program_file = db.program_file(*file);
            let module = parsed_module(db, program_file.python_file(db)).load(db);
            let path = Utf8PathBuf::from(file.path(db).to_string());
            let relative_path = path
                .strip_prefix(&project_root)
                .map_or_else(|_| path.clone(), Utf8Path::to_path_buf);
            sources.push(SourceFile {
                id,
                path,
                relative_path,
                module: module_of(db, *file),
                main_guard: if declarations::has_main_guard(module.syntax()) {
                    MainGuard::Present
                } else {
                    MainGuard::Absent
                },
                exports: declarations::dunder_all_names(module.syntax())
                    .into_iter()
                    .map(SymbolName::new)
                    .collect(),
            });
        }

        self.files = files;
        self.all_files = all_files;
        self.ids = ids;
        self.sources = sources;
        self.references = OnceLock::new();
        Ok(())
    }

    /// The directory ty identified as the project root.
    #[must_use]
    pub fn root(&self) -> &Utf8Path {
        self.db.project().root(&self.db).as_utf8_path()
    }

    fn ty_file(&self, id: FileId) -> Option<File> {
        self.files.get(id.index()).copied()
    }

    /// Builds the reference index now rather than on first use, so callers
    /// can attribute its cost separately.
    pub fn prepare(&self) {
        let _ = self.reference_index();
    }

    fn reference_index(&self) -> &ReferenceIndex {
        self.references.get_or_init(|| {
            let mut index = ReferenceIndex::build(&self.db, &self.all_files);
            index.extend_attribute_names(templates::attribute_names(self.root()));
            index
        })
    }
}

fn is_notebook(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ipynb"))
}

fn module_of(db: &ProjectDatabase, file: File) -> Option<ModulePath> {
    module_name_of(db, file).map(ModulePath::new)
}

/// The dotted module name `file` resolves to on the search paths, if any.
pub(crate) fn module_name_of(db: &dyn ty_python_semantic::Db, file: File) -> Option<String> {
    let program_file = db.program_file(file);
    let resolver_file = program_file.resolver_file(db);
    ty_module_resolver::file_to_module(db, resolver_file)
        .map(|module| module.name(db).as_str().to_owned())
}

impl CodebaseIndex for TyIndex {
    fn files(&self) -> &[SourceFile] {
        &self.sources
    }

    fn symbols(&self, file: FileId) -> Vec<Symbol> {
        let Some(ty_file) = self.ty_file(file) else {
            return Vec::new();
        };
        let program_file = self.db.program_file(ty_file);
        let tree = document_symbols(&self.db, program_file).to_hierarchical();
        let module = parsed_module(&self.db, program_file.python_file(&self.db)).load(&self.db);
        let model = ty_python_semantic::SemanticModel::new(&self.db, program_file);
        let declarations = declarations::declarations_by_name_range(module.syntax(), &model);
        let assigned = declarations::assignment_target_ranges(module.syntax());

        let mut collector = SymbolCollector {
            file,
            tree: &tree,
            declarations,
            out: Vec::new(),
        };
        for (id, info) in tree.iter() {
            collector.visit(id, &info, SymbolScope::Module);
        }
        // A loop or `with` target is a variable to ty, but it is consumed by its
        // own statement; only assignments can be dead.
        collector.out.retain(|symbol| {
            !matches!(symbol.kind, SymbolKind::Variable | SymbolKind::Constant)
                || assigned.contains(&TextRange::new(
                    TextSize::new(symbol.name_span.start().get()),
                    TextSize::new(symbol.name_span.end().get()),
                ))
        });
        collector.out
    }

    fn references(&self, symbol: &Symbol) -> Vec<Reference> {
        let Some(ty_file) = self.ty_file(symbol.file) else {
            return Vec::new();
        };
        let key = DefinitionKey {
            file: ty_file,
            name_range: TextRange::new(
                TextSize::new(symbol.name_span.start().get()),
                TextSize::new(symbol.name_span.end().get()),
            ),
        };

        self.reference_index()
            .uses_of(key)
            .iter()
            .map(|use_site| match self.ids.get(&use_site.file) {
                Some(&file) => Reference::Internal {
                    file,
                    span: span_of(use_site.range),
                },
                None => Reference::External,
            })
            .collect()
    }

    fn imports(&self, file: FileId) -> Vec<Import> {
        let Some(ty_file) = self.ty_file(file) else {
            return Vec::new();
        };
        self.reference_index()
            .imports_from(ty_file)
            .iter()
            .filter_map(|edge| {
                Some(Import {
                    target: *self.ids.get(&edge.target)?,
                    span: span_of(edge.range),
                    kind: edge.kind,
                    names: edge.names,
                })
            })
            .collect()
    }

    fn deletables(&self, file: FileId) -> Vec<Deletable> {
        let Some(ty_file) = self.ty_file(file) else {
            return Vec::new();
        };
        let program_file = self.db.program_file(ty_file);
        let module = parsed_module(&self.db, program_file.python_file(&self.db)).load(&self.db);
        let source = source_text(&self.db, ty_file);
        pyscythe_metrics::deletables(module.syntax(), source.as_str())
    }

    fn source(&self, file: FileId) -> Option<String> {
        let ty_file = self.ty_file(file)?;
        Some(source_text(&self.db, ty_file).as_str().to_owned())
    }

    fn clone_tokens(&self, file: FileId, mode: CloneMode) -> Vec<CloneToken> {
        let Some(ty_file) = self.ty_file(file) else {
            return Vec::new();
        };
        let program_file = self.db.program_file(ty_file);
        let module = parsed_module(&self.db, program_file.python_file(&self.db)).load(&self.db);
        let source = source_text(&self.db, ty_file);
        pyscythe_metrics::clone_tokens(module.tokens(), module.syntax(), source.as_str(), mode)
    }

    fn function_metrics(&self, file: FileId) -> Vec<FunctionMetrics> {
        let Some(ty_file) = self.ty_file(file) else {
            return Vec::new();
        };
        let program_file = self.db.program_file(ty_file);
        let module = parsed_module(&self.db, program_file.python_file(&self.db)).load(&self.db);
        let source = source_text(&self.db, ty_file);
        pyscythe_metrics::measure(module.syntax(), source.as_str())
    }

    fn suppressions(&self, file: FileId) -> Vec<Suppression> {
        let Some(ty_file) = self.ty_file(file) else {
            return Vec::new();
        };
        let program_file = self.db.program_file(ty_file);
        let module = parsed_module(&self.db, program_file.python_file(&self.db)).load(&self.db);
        let source = source_text(&self.db, ty_file);
        let lines = line_index(&self.db, ty_file);
        module
            .tokens()
            .iter()
            .filter(|token| token.kind() == TokenKind::Comment)
            .filter_map(|token| {
                let text = source
                    .as_str()
                    .get(std::ops::Range::<usize>::from(token.range()))?;
                let line = lines.line_column(token.start(), source.as_str()).line;
                let line = Line::from_one_based(u32::try_from(line.get()).ok()?)?;
                Suppression::parse(text, line)
            })
            .collect()
    }

    fn subclass_registration(&self, symbol: &Symbol) -> SubclassRegistration {
        if symbol.kind != SymbolKind::Class {
            return SubclassRegistration::NotRegistered;
        }
        let Some(ty_file) = self.ty_file(symbol.file) else {
            return SubclassRegistration::NotRegistered;
        };
        let hierarchy = inheritance::hierarchy_of_class(
            &self.db,
            ty_file,
            symbol.name_span,
            Some("__init_subclass__"),
        );
        if hierarchy.ancestors.iter().any(|a| a.defines_name) {
            SubclassRegistration::ByBaseHook
        } else {
            SubclassRegistration::NotRegistered
        }
    }

    fn ancestry(&self, symbol: &Symbol) -> Ancestry {
        let Some(ty_file) = self.ty_file(symbol.file) else {
            return Ancestry::unknown();
        };
        let hierarchy = match symbol.kind {
            SymbolKind::Class => {
                inheritance::hierarchy_of_class(&self.db, ty_file, symbol.name_span, None)
            }
            SymbolKind::Method => {
                inheritance::hierarchy_of_enclosing_class(&self.db, ty_file, symbol.name_span, None)
            }
            _ => return Ancestry::unknown(),
        };
        let names = hierarchy
            .ancestors
            .into_iter()
            .map(|ancestor| DottedName::new(ancestor.qualified_name))
            .collect();
        if hierarchy.complete {
            Ancestry::Complete(names)
        } else {
            Ancestry::Incomplete(names)
        }
    }

    fn inheritance(&self, symbol: &Symbol) -> Inheritance {
        if symbol.is_module_level() {
            return Inheritance::Fresh;
        }
        let Some(ty_file) = self.ty_file(symbol.file) else {
            return Inheritance::Fresh;
        };
        if inheritance::overrides_inherited_member(
            &self.db,
            ty_file,
            symbol.full_span,
            symbol.name.as_str(),
        ) {
            Inheritance::OverridesBase
        } else {
            Inheritance::Fresh
        }
    }

    fn external_imports(&self, file: FileId) -> Vec<ExternalImport> {
        let Some(ty_file) = self.ty_file(file) else {
            return Vec::new();
        };
        let records = self.reference_index().external_imports_in(ty_file);
        let mut distributions = match self.distributions.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        records
            .iter()
            .map(|record| {
                let origin = record.site_packages_file.map_or_else(
                    || ImportOrigin::Unresolved,
                    |module_file| ImportOrigin::SitePackages {
                        distributions: distributions.owners(
                            Utf8Path::new(&module_file.path(&self.db).to_string()),
                            &record.top_level,
                        ),
                    },
                );
                ExternalImport {
                    top_level: record.top_level.clone(),
                    span: span_of(record.range),
                    origin,
                }
            })
            .collect()
    }

    fn attribute_name_usage(&self, name: &SymbolName) -> NameUsage {
        if self.reference_index().attribute_name_is_used(name.as_str()) {
            NameUsage::Used
        } else {
            NameUsage::Unused
        }
    }

    fn parameter_name_usage(&self, name: &SymbolName) -> NameUsage {
        if self.reference_index().parameter_name_is_used(name.as_str()) {
            NameUsage::Used
        } else {
            NameUsage::Unused
        }
    }

    fn distribution_requirements(&self, distribution: &DistributionName) -> Vec<DistributionName> {
        let distributions = match self.distributions.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        distributions.requirements_of(distribution)
    }

    fn position(&self, file: FileId, offset: ByteOffset) -> Option<Position> {
        let ty_file = self.ty_file(file)?;
        let source = source_text(&self.db, ty_file);
        let location =
            line_index(&self.db, ty_file).line_column(TextSize::new(offset.get()), source.as_str());
        Some(Position {
            line: Line::from_one_based(u32::try_from(location.line.get()).ok()?)?,
            column: Column::from_one_based(u32::try_from(location.column.get()).ok()?)?,
        })
    }
}

/// Flattens ty's symbol tree into pyscythe symbols, recording each one's parent.
struct SymbolCollector<'a> {
    file: FileId,
    tree: &'a HierarchicalSymbols,
    declarations: FxHashMap<TextRange, declarations::Declaration>,
    out: Vec<Symbol>,
}

impl SymbolCollector<'_> {
    fn visit(&mut self, id: ty_ide::SymbolId, info: &SymbolInfo<'_>, scope: SymbolScope) {
        let Ok(ordinal) = u32::try_from(self.out.len()) else {
            return;
        };
        let symbol_id = SymbolId::new(self.file, ordinal);
        let declaration = self
            .declarations
            .remove(&info.name_range)
            .unwrap_or_default();
        self.out.push(Symbol {
            id: symbol_id,
            file: self.file,
            name: SymbolName::new(info.name.as_ref()),
            kind: kind_of(info.kind),
            scope,
            decorators: declaration.decorators,
            bases: declaration.bases,
            class_keywords: declaration.class_keywords,
            name_span: span_of(info.name_range),
            full_span: span_of(info.full_range),
        });

        let children: Vec<_> = self
            .tree
            .children(id)
            .map(|(child_id, child)| (child_id, child.to_owned_info()))
            .collect();
        for (child_id, child) in children {
            self.visit(child_id, &child, SymbolScope::Nested { parent: symbol_id });
        }
    }
}

/// Lets a borrowed [`SymbolInfo`] outlive the iterator that produced it.
trait ToOwnedInfo {
    fn to_owned_info(&self) -> SymbolInfo<'static>;
}

impl ToOwnedInfo for SymbolInfo<'_> {
    fn to_owned_info(&self) -> SymbolInfo<'static> {
        SymbolInfo {
            name: std::borrow::Cow::Owned(self.name.to_string()),
            kind: self.kind,
            deprecated: self.deprecated,
            imported_from: self.imported_from.clone(),
            name_range: self.name_range,
            full_range: self.full_range,
        }
    }
}

fn span_of(range: TextRange) -> ByteSpan {
    ByteSpan::new(
        ByteOffset::new(range.start().to_u32()),
        ByteOffset::new(range.end().to_u32()),
    )
}

const fn kind_of(kind: ty_ide::SymbolKind) -> SymbolKind {
    match kind {
        ty_ide::SymbolKind::Module => SymbolKind::Module,
        ty_ide::SymbolKind::Class => SymbolKind::Class,
        ty_ide::SymbolKind::Method | ty_ide::SymbolKind::Constructor => SymbolKind::Method,
        ty_ide::SymbolKind::Function => SymbolKind::Function,
        ty_ide::SymbolKind::Variable => SymbolKind::Variable,
        ty_ide::SymbolKind::Constant => SymbolKind::Constant,
        ty_ide::SymbolKind::Property => SymbolKind::Property,
        ty_ide::SymbolKind::Field => SymbolKind::Field,
        ty_ide::SymbolKind::Parameter => SymbolKind::Parameter,
        ty_ide::SymbolKind::TypeParameter => SymbolKind::TypeParameter,
        ty_ide::SymbolKind::Import => SymbolKind::Import,
    }
}
