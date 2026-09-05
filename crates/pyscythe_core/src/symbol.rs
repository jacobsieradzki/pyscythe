//! Named definitions discovered in source files.

use serde::Serialize;

use crate::source::{ByteSpan, FileId};

/// Identifies one symbol: the file it lives in plus its ordinal within that file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolId {
    file: FileId,
    ordinal: u32,
}

impl SymbolId {
    /// Builds an identity for the `ordinal`-th symbol of `file`.
    #[must_use]
    pub const fn new(file: FileId, ordinal: u32) -> Self {
        Self { file, ordinal }
    }
}

/// The name a symbol is bound to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct SymbolName(String);

impl SymbolName {
    /// Wraps a Python identifier.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The identifier as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the name is a dunder such as `__init__` or `__all__`.
    #[must_use]
    pub fn is_dunder(&self) -> bool {
        self.0.len() > 4 && self.0.starts_with("__") && self.0.ends_with("__")
    }

    /// Whether the name follows the leading-underscore private convention.
    #[must_use]
    pub fn is_private(&self) -> bool {
        self.0.starts_with('_') && !self.is_dunder()
    }
}

/// What kind of thing a symbol is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SymbolKind {
    /// A module-level `def`.
    Function,
    /// A `class`.
    Class,
    /// A `def` inside a class body.
    Method,
    /// A plain assignment.
    Variable,
    /// An assignment whose name is `UPPER_CASE`.
    Constant,
    /// A `@property`.
    Property,
    /// An attribute assigned on a class or instance.
    Field,
    /// An `import` or `from ... import` binding.
    Import,
    /// A function or lambda parameter.
    Parameter,
    /// A PEP 695 type parameter.
    TypeParameter,
    /// A module itself.
    Module,
}

/// Where a symbol sits in the file's symbol tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolScope {
    /// Defined directly in the module body.
    Module,
    /// Nested inside another symbol, typically a class.
    Nested {
        /// The enclosing symbol.
        parent: SymbolId,
    },
}

/// A definition in a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// Identity within the index.
    pub id: SymbolId,
    /// The file containing the definition.
    pub file: FileId,
    /// The bound name.
    pub name: SymbolName,
    /// What kind of definition this is.
    pub kind: SymbolKind,
    /// Whether it is module-level or nested.
    pub scope: SymbolScope,
    /// The span of the name token.
    pub name_span: ByteSpan,
    /// The span of the whole definition including its body.
    pub full_span: ByteSpan,
}

impl Symbol {
    /// Whether the symbol is defined directly in the module body.
    #[must_use]
    pub const fn is_module_level(&self) -> bool {
        matches!(self.scope, SymbolScope::Module)
    }
}
