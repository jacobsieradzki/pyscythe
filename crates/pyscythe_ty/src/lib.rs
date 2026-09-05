//! Implements [`CodebaseIndex`] using the ty project database.
//!
//! ty discovers the project, resolves the Python environment, parses every
//! file, and answers semantic questions such as "where is this name used?".
//! This crate only translates between ty's types and pyscythe's domain.

use std::sync::OnceLock;

use camino::{Utf8Path, Utf8PathBuf};
use pyscythe_core::config::{NotebookPolicy, PathPatterns};
use pyscythe_core::index::{CodebaseIndex, Import, Reference};
use pyscythe_core::source::{
    ByteOffset, ByteSpan, Column, FileId, Line, MainGuard, ModulePath, Position, SourceFile,
};
use pyscythe_core::symbol::{Symbol, SymbolId, SymbolKind, SymbolName, SymbolScope};
use ruff_db::files::File;
use ruff_db::parsed::parsed_module;
use ruff_db::source::{line_index, source_text};
use ruff_db::system::{OsSystem, SystemPath};
use ruff_text_size::{TextRange, TextSize};
use rustc_hash::FxHashMap;
use ty_ide::{HierarchicalSymbols, SymbolInfo, document_symbols};
use ty_project::metadata::ProjectMetadataError;
use ty_project::{Db as _, ProjectDatabase, ProjectMetadata};
use ty_python_semantic::Db as _;

use crate::reference_index::{DefinitionKey, ReferenceIndex};

mod declarations;
mod reference_index;

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
    pub fn open(path: &std::path::Path, options: &IndexOptions) -> Result<Self, OpenError> {
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

        let mut metadata = ProjectMetadata::discover(root, &system)?;
        metadata
            .apply_configuration_files(&system)
            .map_err(|error| OpenError::Configuration(error.into()))?;

        let mut db =
            ProjectDatabase::fallible(metadata, system).map_err(OpenError::Configuration)?;
        ruff_db::disable_lru(&mut db);
        db.freeze_open_files();
        db.freeze();

        // Notebooks are scratch space; nobody deletes cells because a linter said so.
        let mut all_files: Vec<File> = db
            .project()
            .files(&db)
            .iter()
            .filter(|file| {
                options.notebooks == NotebookPolicy::Include
                    || !is_notebook(file.path(&db).to_string().as_str())
            })
            .collect();
        all_files.sort_by_key(|file| file.path(&db).to_string());

        let project_root = db.project().root(&db).as_utf8_path().to_path_buf();
        let files: Vec<File> = all_files
            .iter()
            .copied()
            .filter(|file| {
                let path = Utf8PathBuf::from(file.path(&db).to_string());
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
            let module = parsed_module(&db, program_file.python_file(&db)).load(&db);
            sources.push(SourceFile {
                id,
                path: Utf8PathBuf::from(file.path(&db).to_string()),
                module: module_of(&db, *file),
                main_guard: if declarations::has_main_guard(module.syntax()) {
                    MainGuard::Present
                } else {
                    MainGuard::Absent
                },
            });
        }

        Ok(Self {
            db,
            files,
            all_files,
            ids,
            sources,
            references: OnceLock::new(),
        })
    }

    /// The directory ty identified as the project root.
    #[must_use]
    pub fn root(&self) -> &Utf8Path {
        self.db.project().root(&self.db).as_utf8_path()
    }

    fn ty_file(&self, id: FileId) -> Option<File> {
        self.files.get(id.index()).copied()
    }

    fn reference_index(&self) -> &ReferenceIndex {
        self.references
            .get_or_init(|| ReferenceIndex::build(&self.db, &self.all_files))
    }
}

fn is_notebook(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ipynb"))
}

fn module_of(db: &ProjectDatabase, file: File) -> Option<ModulePath> {
    let program_file = db.program_file(file);
    let resolver_file = program_file.resolver_file(db);
    ty_module_resolver::file_to_module(db, resolver_file)
        .map(|module| ModulePath::new(module.name(db).as_str()))
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
        let declarations = declarations::declarations_by_name_range(module.syntax());

        let mut collector = SymbolCollector {
            file,
            tree: &tree,
            declarations,
            out: Vec::new(),
        };
        for (id, info) in tree.iter() {
            collector.visit(id, &info, SymbolScope::Module);
        }
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
                })
            })
            .collect()
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
