//! The port through which analyses see a codebase.

use crate::source::{ByteOffset, ByteSpan, FileId, Position, SourceFile};
use crate::symbol::Symbol;

/// One place a symbol is referred to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reference {
    /// A reference inside a file the index knows about.
    Internal {
        /// The referring file.
        file: FileId,
        /// The span of the referring expression.
        span: ByteSpan,
    },
    /// A reference from a file outside the index, such as site-packages.
    External,
}

/// When an import edge takes effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportKind {
    /// Executed when the importing module loads.
    Runtime,
    /// Inside a function body or a dynamic import by string, so it runs later.
    Deferred,
    /// Under `if TYPE_CHECKING:`, never executed.
    TypeOnly,
}

/// One module importing another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Import {
    /// The imported file.
    pub target: FileId,
    /// Where the import statement (or string) sits in the importing file.
    pub span: ByteSpan,
    /// When the import happens.
    pub kind: ImportKind,
}

/// Everything an analysis may ask about a codebase.
///
/// Implementations resolve names semantically: a reference is only reported
/// when it actually binds to the symbol, not when the text merely matches.
pub trait CodebaseIndex {
    /// Every first-party file, in a stable order.
    fn files(&self) -> &[SourceFile];

    /// The definitions in `file`, in source order.
    fn symbols(&self, file: FileId) -> Vec<Symbol>;

    /// Every reference to `symbol`, excluding its own declaration.
    fn references(&self, symbol: &Symbol) -> Vec<Reference>;

    /// Every module `file` imports, including deferred and type-only imports.
    fn imports(&self, file: FileId) -> Vec<Import>;

    /// Converts a byte offset in `file` to a line and column.
    fn position(&self, file: FileId, offset: ByteOffset) -> Option<Position>;

    /// Looks up a file by identity.
    fn file(&self, id: FileId) -> Option<&SourceFile> {
        self.files().get(id.index())
    }
}
