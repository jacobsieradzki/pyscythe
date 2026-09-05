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

    /// Converts a byte offset in `file` to a line and column.
    fn position(&self, file: FileId, offset: ByteOffset) -> Option<Position>;

    /// Looks up a file by identity.
    fn file(&self, id: FileId) -> Option<&SourceFile> {
        self.files().get(id.index())
    }
}
