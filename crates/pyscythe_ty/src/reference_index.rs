//! A single pass over every project file that resolves each name, attribute,
//! imported symbol, and dotted string to its definition and records where it
//! was used, and that records which module imports which.
//!
//! Building this once is far cheaper than searching the workspace per symbol,
//! and resolving from the use site handles aliased imports uniformly.

use pyscythe_core::index::ImportKind;
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

/// One file importing another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ImportEdge {
    pub(crate) target: File,
    pub(crate) range: TextRange,
    pub(crate) kind: ImportKind,
}

/// Uses of every project definition, the import graph, and every attribute
/// name that is accessed anywhere.
#[derive(Debug, Default)]
pub(crate) struct ReferenceIndex {
    uses: FxHashMap<DefinitionKey, Vec<Use>>,
    imports: FxHashMap<File, Vec<ImportEdge>>,
    attribute_names: FxHashSet<String>,
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
            index.attribute_names.extend(file_uses.attribute_names);
        }
        index
    }

    /// Whether `x.<name>` or `getattr(x, "<name>")` appears anywhere.
    pub(crate) fn attribute_name_is_used(&self, name: &str) -> bool {
        self.attribute_names.contains(name)
    }

    /// Every recorded use of the definition whose name occupies `key`.
    pub(crate) fn uses_of(&self, key: DefinitionKey) -> &[Use] {
        self.uses.get(&key).map_or(&[], Vec::as_slice)
    }

    /// Every import edge leaving `file`.
    pub(crate) fn imports_from(&self, file: File) -> &[ImportEdge] {
        self.imports.get(&file).map_or(&[], Vec::as_slice)
    }
}

#[derive(Debug, Default)]
struct FileUses {
    uses: Vec<(DefinitionKey, Use)>,
    imports: Vec<ImportEdge>,
    attribute_names: FxHashSet<String>,
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

    fn record_import(&mut self, range: TextRange, target: File, kind: ImportKind) {
        if self.project_files.contains(&target) {
            self.out.imports.push(ImportEdge {
                target,
                range,
                kind,
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
                    self.record_import(import_range, module.file(self.db), kind);
                }
            }
        }
    }

    /// Records an edge to `dotted` and every package above it, since importing
    /// `a.b.c` also executes `a/__init__.py` and `a/b/__init__.py`.
    fn record_module_and_ancestors(&mut self, range: TextRange, dotted: &str, level: u32) {
        let kind = self.import_kind();
        let segments: Vec<&str> = dotted.split('.').collect();
        for end in 1..=segments.len() {
            let prefix = segments.get(..end).map(|s| s.join(".")).unwrap_or_default();
            if let Some(module) = self.model.resolve_module(Some(prefix.as_str()), level)
                && let Some(module_file) = module.file(self.db)
            {
                self.record_import(range, module_file, kind);
            }
        }
        if dotted.is_empty()
            && let Some(module) = self.model.resolve_module(None, level)
            && let Some(module_file) = module.file(self.db)
        {
            self.record_import(range, module_file, kind);
        }
    }

    /// A string such as `"pkg.settings.DEBUG"` or `"pkg.cli:main"` counts as a
    /// use of that symbol and a deferred import of its module.
    fn record_string_reference(&mut self, literal: &ast::ExprStringLiteral) {
        let text = literal.value.to_str();
        let Some((module_name, attribute)) = split_dotted_reference(text) else {
            return;
        };
        let Some(module) = self.model.resolve_module(Some(module_name), 0) else {
            return;
        };
        let Some(module_file) = module.file(self.db) else {
            return;
        };
        self.record_import(literal.range(), module_file, ImportKind::Deferred);

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
            AnyNodeRef::StmtImportFrom(import) => {
                let module_name = import.module.as_deref().unwrap_or_default();
                self.record_module_and_ancestors(import.range(), module_name, import.level);
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
                    self.record_module_and_ancestors(alias.range(), alias.name.id.as_str(), 0);
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
