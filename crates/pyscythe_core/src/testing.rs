//! An in-memory [`CodebaseIndex`] for driving analyses in unit tests.

use camino::Utf8PathBuf;

use crate::index::{CodebaseIndex, Reference};
use crate::source::{ByteOffset, ByteSpan, Column, FileId, Line, ModulePath, Position, SourceFile};
use crate::symbol::{Decorator, Symbol, SymbolId, SymbolKind, SymbolName, SymbolScope};

/// Each symbol is laid out on its own "line" of this many bytes so that
/// positions and spans are trivially distinct.
const LINE_STRIDE: u32 = 100;

#[derive(Debug, Default)]
pub(crate) struct FakeIndex {
    files: Vec<SourceFile>,
    symbols: Vec<Symbol>,
    references: Vec<(SymbolId, Reference)>,
}

impl FakeIndex {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn add_file(&mut self, path: &str, module: &str) -> FileId {
        let id = FileId::new(u32::try_from(self.files.len()).expect("fewer than u32::MAX files"));
        self.files.push(SourceFile {
            id,
            path: Utf8PathBuf::from(path),
            module: Some(ModulePath::new(module)),
        });
        id
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

    fn position(&self, _file: FileId, offset: ByteOffset) -> Option<Position> {
        Some(Position {
            line: Line::from_one_based(offset.get() / LINE_STRIDE + 1)?,
            column: Column::from_one_based(offset.get() % LINE_STRIDE + 1)?,
        })
    }
}
