//! A single pass over every project file that resolves each name, attribute,
//! and imported symbol to its definition and records where it was used.
//!
//! Building this once is far cheaper than searching the workspace per symbol,
//! and resolving from the use site handles aliased imports uniformly.

use ruff_db::files::{File, FileRange};
use ruff_db::parsed::parsed_module;
use ruff_python_ast::visitor::source_order::{SourceOrderVisitor, TraversalSignal, walk_body};
use ruff_python_ast::{self as ast, AnyNodeRef};
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::{FxHashMap, FxHashSet};
use ty_project::parallel::{ParallelIteratorExt, minimum_parallel_job_len};
use ty_python_semantic::{
    ImportAliasResolution, ResolvedDefinition, SemanticModel, definitions_for_attribute,
    definitions_for_imported_symbol, definitions_for_name,
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

/// Uses of every project definition, plus which modules were imported.
#[derive(Debug, Default)]
pub(crate) struct ReferenceIndex {
    uses: FxHashMap<DefinitionKey, Vec<Use>>,
    imported_modules: FxHashSet<File>,
}

impl ReferenceIndex {
    /// Walks every file in `files`, in parallel, recording uses of definitions
    /// that live in `files`.
    pub(crate) fn build(db: &dyn ty_project::Db, files: &[File]) -> Self {
        let project_files: FxHashSet<File> = files.iter().copied().collect();
        let minimum_job_len = minimum_parallel_job_len(files.len(), MAX_MIN_FILES_PER_JOB);

        let per_file: Vec<FileUses> = files
            .par_iter()
            .copied()
            .with_min_len(minimum_job_len)
            .map_with_db(db, |db, file| collect_uses(db, file, &project_files))
            .collect();

        let mut index = Self::default();
        for file_uses in per_file {
            for (key, use_site) in file_uses.uses {
                index.uses.entry(key).or_default().push(use_site);
            }
            index.imported_modules.extend(file_uses.imported_modules);
        }
        index
    }

    /// Every recorded use of the definition whose name occupies `key`.
    pub(crate) fn uses_of(&self, key: DefinitionKey) -> &[Use] {
        self.uses.get(&key).map_or(&[], Vec::as_slice)
    }

    /// Whether any file imports `file` as a module.
    #[expect(dead_code, reason = "consumed by the upcoming unused-file analysis")]
    pub(crate) fn is_imported(&self, file: File) -> bool {
        self.imported_modules.contains(&file)
    }
}

#[derive(Debug, Default)]
struct FileUses {
    uses: Vec<(DefinitionKey, Use)>,
    imported_modules: Vec<File>,
}

fn collect_uses(db: &dyn ty_project::Db, file: File, project_files: &FxHashSet<File>) -> FileUses {
    let program_file = db.program_file(file);
    let module = parsed_module(db, program_file.python_file(db)).load(db);
    let model = SemanticModel::new(db, program_file);

    let mut collector = UseCollector {
        db,
        model: &model,
        file,
        project_files,
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
    out: FileUses,
}

impl UseCollector<'_, '_> {
    fn record(&mut self, use_range: TextRange, resolved: Vec<ResolvedDefinition<'_>>) {
        for definition in resolved {
            match definition {
                ResolvedDefinition::Definition(_) | ResolvedDefinition::FileWithRange(_) => {
                    let target: FileRange = definition.focus_range(self.db);
                    if !self.project_files.contains(&target.file()) {
                        continue;
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
                ResolvedDefinition::Module(module) => {
                    let module_file = module.file(self.db);
                    if self.project_files.contains(&module_file) {
                        self.out.imported_modules.push(module_file);
                    }
                }
            }
        }
    }
}

impl<'a> SourceOrderVisitor<'a> for UseCollector<'a, '_> {
    fn enter_node(&mut self, node: AnyNodeRef<'a>) -> TraversalSignal {
        match node {
            AnyNodeRef::ExprName(name) if name.ctx.is_load() => {
                let resolved = definitions_for_name(
                    self.model,
                    name.id.as_str(),
                    node,
                    ImportAliasResolution::ResolveAliases,
                );
                self.record(name.range(), resolved);
            }
            AnyNodeRef::ExprAttribute(attribute) if attribute.ctx.is_load() => {
                let resolved = definitions_for_attribute(self.model, attribute);
                self.record(attribute.attr.range(), resolved);
            }
            AnyNodeRef::StmtImportFrom(import) => {
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
                    self.record(alias.name.range(), resolved);
                }
            }
            AnyNodeRef::StmtImport(import) => {
                for alias in &import.names {
                    let dotted: ast::name::Name = alias.name.id.clone();
                    let resolved = self.model.resolve_module(Some(dotted.as_str()), 0);
                    if let Some(module) = resolved
                        && let Some(module_file) = module.file(self.db)
                        && self.project_files.contains(&module_file)
                    {
                        self.out.imported_modules.push(module_file);
                    }
                }
            }
            _ => {}
        }
        TraversalSignal::Traverse
    }
}
