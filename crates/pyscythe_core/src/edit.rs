//! Source regions that can be removed whole.

use crate::source::{ByteSpan, Column, Line};
use crate::symbol::SymbolName;

/// Whether removing a class-body statement would leave the body empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyAfterRemoval {
    /// Other statements remain.
    StillHasStatements,
    /// The body would be empty, which is a syntax error.
    WouldBeEmpty,
}

/// A definition that can be deleted as a block of whole lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deletable {
    /// The defined name.
    pub name: SymbolName,
    /// Line of the name token.
    pub line: Line,
    /// Column of the name token.
    pub column: Column,
    /// Whole lines from the first decorator to the end of the body, newline included.
    pub lines: ByteSpan,
    /// What the enclosing class body looks like without it; `StillHasStatements` at module level.
    pub body_after_removal: BodyAfterRemoval,
}
