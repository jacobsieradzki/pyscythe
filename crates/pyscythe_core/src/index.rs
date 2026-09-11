//! The port through which analyses see a codebase.

use crate::finding::Rule;
use crate::metrics::FunctionMetrics;
use crate::source::{ByteOffset, ByteSpan, FileId, Line, Position, SourceFile};
use crate::symbol::{DottedName, Symbol, SymbolName};
use crate::tokens::{CloneMode, CloneToken};

/// Whether an attribute name appears anywhere, regardless of what it resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameUsage {
    /// Some `x.name` access, or `getattr(x, "name")`, exists in the project.
    Used,
    /// No such access exists.
    Unused,
}

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

/// Whether a method redefines something a base class already provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Inheritance {
    /// A base class, anywhere in the hierarchy, defines a member of this name.
    OverridesBase,
    /// The name is new on this class.
    Fresh,
}

/// The resolved base classes of a class, transitively, as qualified names
/// such as `sqlalchemy.orm.decl_api.DeclarativeBase`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ancestry {
    /// Every base at every level resolved to a class.
    Complete(Vec<DottedName>),
    /// At least one base could not be resolved, so the list may be missing
    /// ancestors; rules should fall back to what the source says.
    Incomplete(Vec<DottedName>),
}

impl Ancestry {
    /// No bases known at all, as for a symbol that is not a class.
    #[must_use]
    pub const fn unknown() -> Self {
        Self::Incomplete(Vec::new())
    }

    /// The ancestors that did resolve.
    #[must_use]
    pub fn names(&self) -> &[DottedName] {
        match self {
            Self::Complete(names) | Self::Incomplete(names) => names,
        }
    }

    /// Whether every base resolved.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        matches!(self, Self::Complete(_))
    }

    /// Whether any ancestor lives in `package` or one of its submodules.
    #[must_use]
    pub fn has_ancestor_in(&self, package: &str) -> bool {
        self.names().iter().any(|name| {
            name.as_str()
                .strip_prefix(package)
                .is_some_and(|rest| rest.starts_with('.'))
        })
    }
}

/// What a `# pyscythe: ignore` comment covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuppressionScope {
    /// Every rule, for the definition on or below the comment's line.
    AllRules,
    /// Only these rules, for the definition on or below the comment's line.
    Rules(Vec<Rule>),
    /// The whole file: `# pyscythe: ignore-file`.
    File,
}

/// A suppression comment in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suppression {
    /// The line the comment sits on.
    pub line: Line,
    /// What it covers.
    pub scope: SuppressionScope,
}

impl Suppression {
    /// Parses the text after `#` of a comment, such as `pyscythe: ignore[unused-method]`.
    ///
    /// Unknown rule codes are dropped; a comment naming only unknown rules
    /// suppresses nothing.
    #[must_use]
    pub fn parse(comment: &str, line: Line) -> Option<Self> {
        let body = comment.trim_start_matches('#').trim();
        let directive = body.strip_prefix("pyscythe:")?.trim();
        if directive == "ignore-file" {
            return Some(Self {
                line,
                scope: SuppressionScope::File,
            });
        }
        let rest = directive.strip_prefix("ignore")?;
        if rest.trim().is_empty() {
            return Some(Self {
                line,
                scope: SuppressionScope::AllRules,
            });
        }
        let inner = rest.trim().strip_prefix('[')?.split_once(']')?.0;
        let rules: Vec<Rule> = inner
            .split(',')
            .filter_map(|code| Rule::from_code(code.trim()))
            .collect();
        (!rules.is_empty()).then_some(Self {
            line,
            scope: SuppressionScope::Rules(rules),
        })
    }
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

    /// Whether `name` is accessed as an attribute anywhere, by any receiver.
    ///
    /// This is the safety net for duck typing and overriding: a call through a
    /// base type or an untyped object cannot be resolved to one method, but the
    /// name still shows up.
    fn attribute_name_usage(&self, name: &SymbolName) -> NameUsage;

    /// Whether a method or property overrides an inherited member.
    ///
    /// Only meaningful for symbols nested in a class; anything else is `Fresh`.
    fn inheritance(&self, symbol: &Symbol) -> Inheritance;

    /// The transitive base classes of a class symbol, by qualified name.
    ///
    /// Anything that is not a class has unknown ancestry.
    fn ancestry(&self, symbol: &Symbol) -> Ancestry;

    /// Every `# pyscythe: ignore` comment in `file`.
    fn suppressions(&self, file: FileId) -> Vec<Suppression>;

    /// Size and complexity of every function in `file`, in source order.
    fn function_metrics(&self, file: FileId) -> Vec<FunctionMetrics>;

    /// The significant tokens of `file`, normalised for `mode`.
    fn clone_tokens(&self, file: FileId, mode: CloneMode) -> Vec<CloneToken>;

    /// Converts a byte offset in `file` to a line and column.
    fn position(&self, file: FileId, offset: ByteOffset) -> Option<Position>;

    /// Looks up a file by identity.
    fn file(&self, id: FileId) -> Option<&SourceFile> {
        self.files().get(id.index())
    }
}

#[cfg(test)]
mod tests {
    use super::{Suppression, SuppressionScope};
    use crate::finding::Rule;
    use crate::source::Line;

    fn line() -> Line {
        Line::from_one_based(7).unwrap()
    }

    #[test]
    fn parses_bare_ignore() {
        let suppression = Suppression::parse("# pyscythe: ignore", line()).unwrap();
        assert_eq!(suppression.scope, SuppressionScope::AllRules);
    }

    #[test]
    fn parses_rule_lists() {
        let suppression =
            Suppression::parse("#pyscythe:ignore[unused-method, unused-class]", line()).unwrap();
        assert_eq!(
            suppression.scope,
            SuppressionScope::Rules(vec![Rule::UnusedMethod, Rule::UnusedClass])
        );
    }

    #[test]
    fn parses_ignore_file() {
        let suppression = Suppression::parse("# pyscythe: ignore-file", line()).unwrap();
        assert_eq!(suppression.scope, SuppressionScope::File);
    }

    #[test]
    fn ignores_other_comments_and_unknown_rules() {
        assert!(Suppression::parse("# noqa", line()).is_none());
        assert!(Suppression::parse("# pyscythe: ignore[not-a-rule]", line()).is_none());
        assert!(Suppression::parse("# pyscythe: something", line()).is_none());
    }
}
