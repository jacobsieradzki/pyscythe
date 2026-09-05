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
        }
    }
}

/// How sure the analysis is that acting on the finding is safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Confidence {
    /// Almost certainly dead; nothing outside the file could reach it.
    High,
    /// Probably dead, but public names may be consumed by code we cannot see.
    Medium,
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
    /// The symbol concerned.
    pub symbol: SymbolName,
    /// Where the symbol's name appears.
    pub position: Option<Position>,
    /// How confident the analysis is.
    pub confidence: Confidence,
    /// A one-line human explanation.
    pub message: String,
}
