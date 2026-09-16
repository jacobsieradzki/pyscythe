//! An in-memory [`CodebaseIndex`] for driving analyses in unit tests.

use camino::Utf8PathBuf;

use crate::edit::{BodyAfterRemoval, Deletable};
use crate::metrics::FunctionMetrics;
use crate::tokens::{CloneMode, CloneToken};

use crate::index::{
    Ancestry, CodebaseIndex, ExternalImport, Import, ImportCondition, ImportKind, ImportOrigin,
    ImportedNames, Inheritance, NameUsage, Reference, SubclassRegistration, Suppression,
    SuppressionScope,
};
use crate::manifest::DistributionName;
use crate::source::{
    ByteOffset, ByteSpan, Column, FileId, Line, MainGuard, ModulePath, Position, SourceFile,
};
use crate::symbol::{Decorator, Symbol, SymbolId, SymbolKind, SymbolName, SymbolScope};

/// Each symbol is laid out on its own "line" of this many bytes so that
/// positions and spans are trivially distinct.
const LINE_STRIDE: u32 = 100;

#[derive(Debug, Default)]
pub(crate) struct FakeIndex {
    files: Vec<SourceFile>,
    symbols: Vec<Symbol>,
    references: Vec<(SymbolId, Reference)>,
    imports: Vec<(FileId, Import)>,
    used_attribute_names: Vec<String>,
    overriding: Vec<SymbolId>,
    ancestries: Vec<(SymbolId, Ancestry)>,
    suppressions: Vec<(FileId, Suppression)>,
    metrics: Vec<(FileId, FunctionMetrics)>,
    tokens: Vec<(FileId, Vec<CloneToken>)>,
    sources: Vec<(FileId, String, Vec<Deletable>)>,
    external_imports: Vec<(FileId, ExternalImport)>,
    parameter_names: Vec<String>,
    requirements: Vec<(DistributionName, DistributionName)>,
    path_literals: Vec<String>,
}

impl FakeIndex {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn add_file(&mut self, path: &str, module: &str) -> FileId {
        self.push_file(path, module, MainGuard::Absent)
    }

    /// A file with an `if __name__ == "__main__":` guard.
    pub(crate) fn add_script(&mut self, path: &str, module: &str) -> FileId {
        self.push_file(path, module, MainGuard::Present)
    }

    fn push_file(&mut self, path: &str, module: &str, main_guard: MainGuard) -> FileId {
        let id = FileId::new(u32::try_from(self.files.len()).expect("fewer than u32::MAX files"));
        let relative_path: Utf8PathBuf = Utf8PathBuf::from(path).components().skip(2).collect();
        self.files.push(SourceFile {
            id,
            path: Utf8PathBuf::from(path),
            relative_path,
            module: Some(ModulePath::new(module)),
            main_guard,
            exports: Vec::new(),
        });
        id
    }

    /// Sets the names listed in `file`'s `__all__`.
    pub(crate) fn set_exports(&mut self, file: FileId, names: &[&str]) {
        if let Some(source) = self.files.iter_mut().find(|f| f.id == file) {
            source.exports = names.iter().map(|n| SymbolName::new(*n)).collect();
        }
    }

    pub(crate) fn add_import(&mut self, from: FileId, to: FileId, kind: ImportKind) {
        self.add_import_of(from, to, kind, ImportedNames::Explicit);
    }

    /// `from to import *` at module level of `from`.
    pub(crate) fn add_wildcard_import(&mut self, from: FileId, to: FileId) {
        self.add_import_of(from, to, ImportKind::Runtime, ImportedNames::Wildcard);
    }

    fn add_import_of(&mut self, from: FileId, to: FileId, kind: ImportKind, names: ImportedNames) {
        let ordinal = u32::try_from(self.imports.len()).expect("few imports");
        self.imports.push((
            from,
            Import {
                target: to,
                span: ByteSpan::new(ByteOffset::new(ordinal), ByteOffset::new(ordinal + 1)),
                kind,
                names,
            },
        ));
    }

    pub(crate) fn add_symbol(&mut self, file: FileId, name: &str, kind: SymbolKind) -> SymbolId {
        self.push_symbol(file, name, kind, SymbolScope::Module, Vec::new())
    }

    pub(crate) fn add_decorated_symbol(
        &mut self,
        file: FileId,
        name: &str,
        kind: SymbolKind,
        decorators: &[&str],
    ) -> SymbolId {
        let decorators = decorators.iter().map(|d| Decorator::named(*d)).collect();
        self.push_symbol(file, name, kind, SymbolScope::Module, decorators)
    }

    pub(crate) fn add_nested_symbol(
        &mut self,
        file: FileId,
        parent: SymbolId,
        name: &str,
        kind: SymbolKind,
    ) -> SymbolId {
        self.push_symbol(file, name, kind, SymbolScope::Nested { parent }, Vec::new())
    }

    pub(crate) fn add_decorated_nested_symbol(
        &mut self,
        file: FileId,
        parent: SymbolId,
        name: &str,
        kind: SymbolKind,
        decorators: &[&str],
    ) -> SymbolId {
        let decorators = decorators.iter().map(|d| Decorator::named(*d)).collect();
        self.push_symbol(file, name, kind, SymbolScope::Nested { parent }, decorators)
    }

    fn symbol(&self, id: SymbolId) -> &Symbol {
        self.symbols
            .iter()
            .find(|s| s.id == id)
            .expect("symbol was added")
    }

    fn line_of(offset: ByteOffset) -> Line {
        Line::from_one_based(offset.get() / LINE_STRIDE + 1).expect("non-zero line")
    }

    pub(crate) fn suppress_on_name_line(&mut self, symbol: SymbolId, scope: SuppressionScope) {
        let target = self.symbol(symbol);
        let line = Self::line_of(target.name_span.start());
        self.suppressions
            .push((target.file, Suppression { line, scope }));
    }

    pub(crate) fn suppress_on_line_above(&mut self, symbol: SymbolId, scope: SuppressionScope) {
        let target = self.symbol(symbol);
        let start = Self::line_of(target.full_span.start());
        let line = Line::from_one_based(start.get() - 1).expect("symbol is not on line one");
        self.suppressions
            .push((target.file, Suppression { line, scope }));
    }

    pub(crate) fn suppress_file(&mut self, file: FileId) {
        self.suppressions.push((
            file,
            Suppression {
                line: Line::from_one_based(1).expect("one"),
                scope: SuppressionScope::File,
            },
        ));
    }

    /// A string literal somewhere names a `.py` file.
    pub(crate) fn add_path_literal(&mut self, path: &str) {
        self.path_literals.push(path.to_owned());
    }

    /// A symbol with one fully described decorator.
    pub(crate) fn add_symbol_decorated_by(
        &mut self,
        file: FileId,
        name: &str,
        kind: SymbolKind,
        decorator: Decorator,
    ) -> SymbolId {
        self.push_symbol(file, name, kind, SymbolScope::Module, vec![decorator])
    }

    /// A function somewhere takes a parameter called `name`.
    pub(crate) fn add_parameter_name(&mut self, name: &str) {
        self.parameter_names.push(name.to_owned());
    }

    /// The installed `distribution` declares `requires` in its metadata.
    pub(crate) fn add_requirement(&mut self, distribution: &str, requires: &str) {
        self.requirements.push((
            DistributionName::normalize(distribution),
            DistributionName::normalize(requires),
        ));
    }

    pub(crate) fn add_external_import(
        &mut self,
        file: FileId,
        top_level: &str,
        origin: ImportOrigin,
    ) {
        self.add_conditional_import(file, top_level, origin, ImportCondition::Always);
    }

    pub(crate) fn add_conditional_import(
        &mut self,
        file: FileId,
        top_level: &str,
        origin: ImportOrigin,
        condition: ImportCondition,
    ) {
        let ordinal = u32::try_from(self.external_imports.len()).expect("few imports");
        self.external_imports.push((
            file,
            ExternalImport {
                top_level: top_level.to_owned(),
                span: ByteSpan::new(ByteOffset::new(ordinal), ByteOffset::new(ordinal + 1)),
                origin,
                condition,
            },
        ));
    }

    /// Attaches real source text to `file` and derives deletables from it with a
    /// tiny reader: every `def name` or `class name` line starts a block that
    /// runs until the next non-indented, non-blank line. Symbol positions are
    /// rewritten to match the source so findings line up with deletables.
    pub(crate) fn set_source_with_deletables(&mut self, file: FileId, source: &str) {
        let lines: Vec<&str> = source.split_inclusive('\n').collect();
        let mut offsets = Vec::with_capacity(lines.len() + 1);
        let mut offset = 0u32;
        for line in &lines {
            offsets.push(offset);
            offset += u32::try_from(line.len()).expect("short file");
        }
        offsets.push(offset);

        let mut deletables = Vec::new();
        let mut index = 0;
        while index < lines.len() {
            let line = lines[index];
            let indent = line.len() - line.trim_start().len();
            let word = line
                .trim_start()
                .split([' ', '(', ':'])
                .next()
                .unwrap_or("");
            let name = line.trim_start()[word.len()..]
                .trim_start()
                .split(['(', ':'])
                .next()
                .unwrap_or("")
                .to_owned();
            if !matches!(word, "def" | "class") {
                index += 1;
                continue;
            }
            let mut end = index + 1;
            while end < lines.len()
                && (lines[end].trim().is_empty()
                    || lines[end].len() - lines[end].trim_start().len() > indent)
            {
                end += 1;
            }
            while end > index + 1 && lines[end - 1].trim().is_empty() {
                end -= 1;
            }
            let column = u32::try_from(indent + word.len() + 2).expect("short line");
            let line_number = Line::from_one_based(u32::try_from(index + 1).expect("few lines"))
                .expect("non-zero");
            let start = offsets[index];
            let name_start = start + column - 1;
            if let Some(symbol) = self
                .symbols
                .iter_mut()
                .find(|s| s.file == file && s.name.as_str() == name)
            {
                symbol.name_span = ByteSpan::new(
                    ByteOffset::new(name_start),
                    ByteOffset::new(name_start + u32::try_from(name.len()).expect("short")),
                );
                symbol.full_span =
                    ByteSpan::new(ByteOffset::new(start), ByteOffset::new(offsets[end]));
            }
            let body_after_removal = if indent > 0 {
                let siblings = lines[..index]
                    .iter()
                    .chain(lines[end..].iter())
                    .filter(|l| l.len() - l.trim_start().len() == indent && !l.trim().is_empty())
                    .count();
                if siblings == 0 {
                    BodyAfterRemoval::WouldBeEmpty
                } else {
                    BodyAfterRemoval::StillHasStatements
                }
            } else {
                BodyAfterRemoval::StillHasStatements
            };
            deletables.push(Deletable {
                name: SymbolName::new(&name),
                line: line_number,
                column: Column::from_one_based(column).expect("non-zero"),
                lines: ByteSpan::new(ByteOffset::new(start), ByteOffset::new(offsets[end])),
                body_after_removal,
            });
            // Keep scanning inside the block so nested definitions get their own entry.
            index += 1;
        }
        self.sources.push((file, source.to_owned(), deletables));
    }

    /// Gives `file` one token per whitespace-separated word of `source`, each
    /// line of `source` on its own line.
    pub(crate) fn set_tokens(&mut self, file: FileId, source: &str) {
        let tokens = source
            .lines()
            .enumerate()
            .flat_map(|(index, line)| {
                let number = Line::from_one_based(u32::try_from(index + 1).expect("few lines"))
                    .expect("non-zero");
                line.split_whitespace()
                    .map(move |word| CloneToken {
                        text: word.to_owned(),
                        line: number,
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        self.tokens.push((file, tokens));
    }

    pub(crate) fn add_function_metrics(
        &mut self,
        file: FileId,
        name: &str,
        cyclomatic: u32,
        cognitive: u32,
        lines: u32,
        parameters: u32,
    ) {
        let ordinal = u32::try_from(self.metrics.len()).expect("few functions");
        let start = ordinal * LINE_STRIDE + 4;
        self.metrics.push((
            file,
            FunctionMetrics {
                name: SymbolName::new(name),
                owner: None,
                name_span: ByteSpan::new(ByteOffset::new(start), ByteOffset::new(start + 1)),
                lines,
                parameters,
                cyclomatic,
                cognitive,
                max_nesting: 0,
            },
        ));
    }

    /// Records the resolved ancestors of a class.
    pub(crate) fn set_ancestry(&mut self, symbol: SymbolId, ancestry: Ancestry) {
        self.ancestries.push((symbol, ancestry));
    }

    /// Records that `symbol` overrides a member of a base class.
    pub(crate) fn mark_overrides_base(&mut self, symbol: SymbolId) {
        self.overriding.push(symbol);
    }

    /// Records that `x.<name>` appears somewhere, without resolving it.
    pub(crate) fn mark_attribute_name_used(&mut self, name: &str) {
        self.used_attribute_names.push(name.to_owned());
    }

    fn push_symbol(
        &mut self,
        file: FileId,
        name: &str,
        kind: SymbolKind,
        scope: SymbolScope,
        decorators: Vec<Decorator>,
    ) -> SymbolId {
        let ordinal = u32::try_from(self.symbols.iter().filter(|s| s.file == file).count())
            .expect("fewer than u32::MAX symbols");
        let id = SymbolId::new(file, ordinal);
        let line_start = ordinal * LINE_STRIDE;
        let name_len = u32::try_from(name.len()).expect("short names");
        self.symbols.push(Symbol {
            id,
            file,
            name: SymbolName::new(name),
            kind,
            scope,
            decorators,
            bases: Vec::new(),
            class_keywords: Vec::new(),
            name_span: ByteSpan::new(
                ByteOffset::new(line_start + 4),
                ByteOffset::new(line_start + 4 + name_len),
            ),
            full_span: ByteSpan::new(
                ByteOffset::new(line_start),
                ByteOffset::new(line_start + LINE_STRIDE - 1),
            ),
        });
        id
    }

    /// Adds a reference to `symbol` from somewhere in `from`, outside any definition.
    pub(crate) fn add_reference(&mut self, symbol: SymbolId, from: FileId) {
        let far_away = ByteOffset::new(u32::MAX - 10);
        self.references.push((
            symbol,
            Reference::Internal {
                file: from,
                span: ByteSpan::new(far_away, ByteOffset::new(u32::MAX - 5)),
            },
        ));
    }

    pub(crate) fn add_reference_inside_own_body(&mut self, symbol: SymbolId) {
        let target = self
            .symbols
            .iter()
            .find(|s| s.id == symbol)
            .expect("symbol was added");
        let inside = ByteOffset::new(target.full_span.start().get() + 20);
        self.references.push((
            symbol,
            Reference::Internal {
                file: target.file,
                span: ByteSpan::new(inside, ByteOffset::new(inside.get() + 5)),
            },
        ));
    }

    pub(crate) fn add_external_reference(&mut self, symbol: SymbolId) {
        self.references.push((symbol, Reference::External));
    }
}

impl CodebaseIndex for FakeIndex {
    fn files(&self) -> &[SourceFile] {
        &self.files
    }

    fn symbols(&self, file: FileId) -> Vec<Symbol> {
        self.symbols
            .iter()
            .filter(|s| s.file == file)
            .cloned()
            .collect()
    }

    fn references(&self, symbol: &Symbol) -> Vec<Reference> {
        self.references
            .iter()
            .filter(|(id, _)| *id == symbol.id)
            .map(|(_, reference)| *reference)
            .collect()
    }

    fn imports(&self, file: FileId) -> Vec<Import> {
        self.imports
            .iter()
            .filter(|(from, _)| *from == file)
            .map(|(_, import)| *import)
            .collect()
    }

    fn external_imports(&self, file: FileId) -> Vec<ExternalImport> {
        self.external_imports
            .iter()
            .filter(|(f, _)| *f == file)
            .map(|(_, i)| i.clone())
            .collect()
    }

    fn attribute_name_usage(&self, name: &SymbolName) -> NameUsage {
        if self.used_attribute_names.iter().any(|n| n == name.as_str()) {
            NameUsage::Used
        } else {
            NameUsage::Unused
        }
    }

    fn parameter_name_usage(&self, name: &SymbolName) -> NameUsage {
        if self.parameter_names.iter().any(|n| n == name.as_str()) {
            NameUsage::Used
        } else {
            NameUsage::Unused
        }
    }

    fn path_literals(&self) -> Vec<String> {
        self.path_literals.clone()
    }

    fn distribution_requirements(&self, distribution: &DistributionName) -> Vec<DistributionName> {
        self.requirements
            .iter()
            .filter(|(from, _)| from == distribution)
            .map(|(_, requires)| requires.clone())
            .collect()
    }

    fn deletables(&self, file: FileId) -> Vec<Deletable> {
        self.sources
            .iter()
            .find(|(f, _, _)| *f == file)
            .map(|(_, _, d)| d.clone())
            .unwrap_or_default()
    }

    fn source(&self, file: FileId) -> Option<String> {
        self.sources
            .iter()
            .find(|(f, _, _)| *f == file)
            .map(|(_, s, _)| s.clone())
    }

    fn clone_tokens(&self, file: FileId, _mode: CloneMode) -> Vec<CloneToken> {
        self.tokens
            .iter()
            .find(|(f, _)| *f == file)
            .map(|(_, tokens)| tokens.clone())
            .unwrap_or_default()
    }

    fn function_metrics(&self, file: FileId) -> Vec<FunctionMetrics> {
        self.metrics
            .iter()
            .filter(|(f, _)| *f == file)
            .map(|(_, m)| m.clone())
            .collect()
    }

    fn suppressions(&self, file: FileId) -> Vec<Suppression> {
        self.suppressions
            .iter()
            .filter(|(f, _)| *f == file)
            .map(|(_, s)| s.clone())
            .collect()
    }

    fn subclass_registration(&self, _symbol: &Symbol) -> SubclassRegistration {
        SubclassRegistration::NotRegistered
    }

    fn ancestry(&self, symbol: &Symbol) -> Ancestry {
        self.ancestries
            .iter()
            .find(|(id, _)| *id == symbol.id)
            .map_or_else(Ancestry::unknown, |(_, ancestry)| ancestry.clone())
    }

    fn inheritance(&self, symbol: &Symbol) -> Inheritance {
        if self.overriding.contains(&symbol.id) {
            Inheritance::OverridesBase
        } else {
            Inheritance::Fresh
        }
    }

    fn position(&self, file: FileId, offset: ByteOffset) -> Option<Position> {
        if let Some((_, source, _)) = self.sources.iter().find(|(f, _, _)| *f == file) {
            let before = source.get(..offset.get() as usize)?;
            let line = u32::try_from(before.matches('\n').count() + 1).ok()?;
            let column = u32::try_from(before.rsplit('\n').next().map_or(0, str::len) + 1).ok()?;
            return Some(Position {
                line: Line::from_one_based(line)?,
                column: Column::from_one_based(column)?,
            });
        }
        Some(Position {
            line: Line::from_one_based(offset.get() / LINE_STRIDE + 1)?,
            column: Column::from_one_based(offset.get() % LINE_STRIDE + 1)?,
        })
    }
}
