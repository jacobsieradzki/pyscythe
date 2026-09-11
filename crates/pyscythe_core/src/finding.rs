//! Things an analysis wants a human or a machine to act on.

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::source::{ModulePath, Position};
use crate::symbol::{SymbolKind, SymbolName};

/// The rule a finding was produced by.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum Rule {
    /// A module-level function nothing refers to.
    UnusedFunction,
    /// A class nothing refers to.
    UnusedClass,
    /// A module-level variable or constant nothing refers to.
    UnusedVariable,
    /// A method or property nothing calls, by resolution or by name.
    UnusedMethod,
    /// A module no other module imports and nothing runs.
    UnusedFile,
    /// A set of modules that import each other at load time.
    CircularImport,
    /// A `# pyscythe: ignore` comment that silences nothing.
    UnusedSuppression,
    /// A function whose complexity is over the threshold.
    ComplexFunction,
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

    /// Every rule, in a stable order for tooling metadata.
    pub const ALL: [Self; 8] = [
        Self::UnusedFunction,
        Self::UnusedClass,
        Self::UnusedVariable,
        Self::UnusedMethod,
        Self::UnusedFile,
        Self::CircularImport,
        Self::UnusedSuppression,
        Self::ComplexFunction,
    ];

    /// The kebab-case identifier used in JSON, SARIF, and suppression comments.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnusedFunction => "unused-function",
            Self::UnusedClass => "unused-class",
            Self::UnusedVariable => "unused-variable",
            Self::UnusedMethod => "unused-method",
            Self::UnusedFile => "unused-file",
            Self::CircularImport => "circular-import",
            Self::UnusedSuppression => "unused-suppression",
            Self::ComplexFunction => "complex-function",
        }
    }

    /// Parses a rule from its code.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|rule| rule.code() == code)
    }

    /// A one-line description for tooling metadata.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::UnusedFunction => "A module-level function that nothing refers to.",
            Self::UnusedClass => "A class that nothing refers to.",
            Self::UnusedVariable => "A module-level variable or constant that nothing refers to.",
            Self::UnusedMethod => {
                "A method or property that nothing calls, by resolution or by name."
            }
            Self::UnusedFile => "A module that no other module imports and nothing runs.",
            Self::CircularImport => "Modules that import each other at load time.",
            Self::UnusedSuppression => "A `# pyscythe: ignore` comment that silences nothing.",
            Self::ComplexFunction => {
                "A function whose cyclomatic or cognitive complexity is over the threshold."
            }
        }
    }

    /// The noun used in human output, such as "function".
    #[must_use]
    pub const fn noun(self) -> &'static str {
        match self {
            Self::UnusedFunction | Self::ComplexFunction => "function",
            Self::UnusedClass => "class",
            Self::UnusedVariable => "variable",
            Self::UnusedMethod => "method",
            Self::UnusedFile => "file",
            Self::CircularImport => "import cycle",
            Self::UnusedSuppression => "suppression comment",
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
    /// Plausible, but overriding, duck typing, or reflection could hide a use.
    Low,
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
        /// The class that owns it, for methods and properties.
        #[serde(skip_serializing_if = "Option::is_none")]
        owner: Option<SymbolName>,
    },
    /// A chain of imports that returns to its start.
    Cycle {
        /// Each link imports the next; the last imports the first.
        chain: Vec<Location>,
    },
    /// A whole file.
    File,
    /// A comment, located by its position alone.
    Comment,
    /// Measurements of a function.
    Metrics {
        /// The function, qualified by its class for methods.
        function: String,
        /// `McCabe` cyclomatic complexity.
        cyclomatic: u32,
        /// Cognitive complexity.
        cognitive: u32,
        /// Source lines in the definition.
        lines: u32,
        /// Parameter count.
        parameters: u32,
        /// Deepest control-flow nesting.
        max_nesting: u32,
    },
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
            Detail::Symbol { symbol, .. } => Some(symbol),
            Detail::Cycle { .. } | Detail::File | Detail::Comment | Detail::Metrics { .. } => None,
        }
    }
}
