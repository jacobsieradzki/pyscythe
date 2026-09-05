//! Things an analysis wants a human or a machine to act on.

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::source::{ModulePath, Position};
use crate::symbol::{SymbolKind, SymbolName};

/// The rule a finding was produced by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Rule {
    /// A module-level function nothing refers to.
    UnusedFunction,
    /// A class nothing refers to.
    UnusedClass,
    /// A module-level variable or constant nothing refers to.
    UnusedVariable,
    /// A module no other module imports and nothing runs.
    UnusedFile,
    /// A set of modules that import each other at load time.
    CircularImport,
}

impl Rule {
    /// The dead-code rule that applies to a symbol kind, if any.
    #[must_use]
    pub const fn for_unused(kind: SymbolKind) -> Option<Self> {
        match kind {
            SymbolKind::Function => Some(Self::UnusedFunction),
            SymbolKind::Class => Some(Self::UnusedClass),
            SymbolKind::Variable | SymbolKind::Constant => Some(Self::UnusedVariable),
            SymbolKind::Method
            | SymbolKind::Property
            | SymbolKind::Field
            | SymbolKind::Import
            | SymbolKind::Parameter
            | SymbolKind::TypeParameter
            | SymbolKind::Module => None,
        }
    }

    /// The noun used in human output, such as "function".
    #[must_use]
    pub const fn noun(self) -> &'static str {
        match self {
            Self::UnusedFunction => "function",
            Self::UnusedClass => "class",
            Self::UnusedVariable => "variable",
            Self::UnusedFile => "file",
            Self::CircularImport => "import cycle",
        }
    }
}

/// How sure the analysis is that acting on the finding is safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Confidence {
    /// Almost certainly right; nothing outside the project could change it.
    High,
    /// Probably right, but public names may be consumed by code we cannot see.
    Medium,
}

/// A place in the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Location {
    /// Absolute path of the file.
    pub path: Utf8PathBuf,
    /// Module path of the file, when resolvable.
    pub module: Option<ModulePath>,
    /// Line and column, when resolvable.
    pub position: Option<Position>,
}

/// What a finding is about, beyond its location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Detail {
    /// A named definition.
    Symbol {
        /// The symbol concerned.
        symbol: SymbolName,
    },
    /// A chain of imports that returns to its start.
    Cycle {
        /// Each link imports the next; the last imports the first.
        chain: Vec<Location>,
    },
    /// A whole file.
    File,
}

/// One actionable result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// The rule that fired.
    pub rule: Rule,
    /// Absolute path of the file.
    pub path: Utf8PathBuf,
    /// Module path of the file, when resolvable.
    pub module: Option<ModulePath>,
    /// Where in the file the finding points.
    pub position: Option<Position>,
    /// How confident the analysis is.
    pub confidence: Confidence,
    /// A one-line human explanation.
    pub message: String,
    /// What the finding is about.
    #[serde(flatten)]
    pub detail: Detail,
}

impl Finding {
    /// The symbol this finding is about, for symbol findings.
    #[must_use]
    pub const fn symbol(&self) -> Option<&SymbolName> {
        match &self.detail {
            Detail::Symbol { symbol } => Some(symbol),
            Detail::Cycle { .. } | Detail::File => None,
        }
    }
}
