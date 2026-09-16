//! Token streams prepared for clone detection.

use crate::source::Line;

/// How aggressively tokens are normalised before comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloneMode {
    /// Tokens must match exactly.
    Strict,
    /// Identifiers are interchangeable: renamed copies still match.
    Mild,
    /// Identifiers and literals are interchangeable: only structure matters.
    Weak,
}

/// One significant token, normalised according to a [`CloneMode`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CloneToken {
    /// The comparison key: source text, or a placeholder when normalised.
    pub text: String,
    /// The line the token starts on.
    pub line: Line,
    /// Where the token sits relative to a statement.
    pub nesting: Nesting,
}

/// Whether a token stands at statement level or inside an expression.
///
/// A run of tokens that never reaches statement level is a fragment of one
/// call's arguments or one collection literal. With identifiers normalised,
/// every such run looks like every other, so matching one says nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Nesting {
    /// Outside every bracket.
    Statement,
    /// Inside a bracket, so part of a single expression.
    Expression,
}
