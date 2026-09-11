//! Measurements of one function, as produced by a metrics adapter.

use crate::source::ByteSpan;
use crate::symbol::SymbolName;

/// Size and complexity of a single `def`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionMetrics {
    /// The function's name.
    pub name: SymbolName,
    /// The class that owns it, for methods.
    pub owner: Option<SymbolName>,
    /// The span of the name token.
    pub name_span: ByteSpan,
    /// Source lines from `def` to the last line of the body, decorators excluded.
    pub lines: u32,
    /// Parameters, `self` and `cls` included.
    pub parameters: u32,
    /// `McCabe` cyclomatic complexity: one plus the number of decision points.
    pub cyclomatic: u32,
    /// Cognitive complexity in the `SonarSource` sense: decision points weighted by nesting.
    pub cognitive: u32,
    /// Deepest nesting of control flow inside the body.
    pub max_nesting: u32,
}

impl FunctionMetrics {
    /// `Owner.name` for methods, `name` otherwise.
    #[must_use]
    pub fn qualified_name(&self) -> String {
        self.owner.as_ref().map_or_else(
            || self.name.as_str().to_owned(),
            |owner| format!("{}.{}", owner.as_str(), self.name.as_str()),
        )
    }
}
