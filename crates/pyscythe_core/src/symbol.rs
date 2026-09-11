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
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, serde::Deserialize)]
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

/// A dotted name as written in source, such as `router.get` or `pytest.fixture`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DottedName(String);

impl DottedName {
    /// Wraps a dotted name.
    #[must_use]
    pub fn new(dotted: impl Into<String>) -> Self {
        Self(dotted.into())
    }

    /// The dotted name as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The segments between the dots.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('.')
    }

    /// The final segment, such as `get` in `router.get`.
    #[must_use]
    pub fn last_segment(&self) -> &str {
        self.0.rsplit('.').next().unwrap_or(&self.0)
    }

    /// Whether the name has a receiver, such as `router` in `router.get`.
    #[must_use]
    pub fn has_receiver(&self) -> bool {
        self.0.contains('.')
    }

    /// Whether the name is exactly `text`.
    #[must_use]
    pub fn is(&self, text: &str) -> bool {
        self.0 == text
    }
}

/// A keyword argument name passed to a decorator, such as `autouse`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeywordName(String);

impl KeywordName {
    /// Wraps a keyword argument name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The keyword as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A decorator applied to a definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decorator {
    /// The decorator expression with any call arguments stripped: `@router.get("/")` is `router.get`.
    pub name: DottedName,
    /// Keyword argument names passed to the decorator call, if it was called.
    pub keywords: Vec<KeywordName>,
    /// The module that defines the decorator, when it could be resolved:
    /// `fastapi.routing` for `@router.get`, regardless of what `router` is called.
    pub module: Option<crate::source::ModulePath>,
}

impl Decorator {
    /// A decorator with no call arguments and no resolved origin.
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: DottedName::new(name),
            keywords: Vec::new(),
            module: None,
        }
    }

    /// The same decorator, known to come from `module`.
    #[must_use]
    pub fn from_module(mut self, module: impl Into<String>) -> Self {
        self.module = Some(crate::source::ModulePath::new(module));
        self
    }

    /// Whether the decorator is defined in `package` or a submodule of it, or
    /// could not be resolved at all (in which case the name has to do).
    #[must_use]
    pub fn comes_from_any(&self, packages: &[&str]) -> bool {
        self.module.as_ref().is_none_or(|module| {
            packages.iter().any(|package| {
                module.as_str() == *package
                    || module
                        .as_str()
                        .strip_prefix(package)
                        .is_some_and(|rest| rest.starts_with('.'))
            })
        })
    }

    /// Whether the decorator call passed a keyword argument named `keyword`.
    #[must_use]
    pub fn has_keyword(&self, keyword: &str) -> bool {
        self.keywords.iter().any(|k| k.as_str() == keyword)
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
    /// Decorators applied to the definition, outermost first.
    pub decorators: Vec<Decorator>,
    /// For classes, the base classes as written, such as `Base` or `db.Model`.
    pub bases: Vec<DottedName>,
    /// For classes, keyword arguments in the class statement, such as `table` in `table=True`.
    pub class_keywords: Vec<KeywordName>,
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

    /// Whether any decorator satisfies `predicate`.
    pub fn has_decorator(&self, predicate: impl Fn(&Decorator) -> bool) -> bool {
        self.decorators.iter().any(predicate)
    }

    /// Whether any base class's last segment is one of `names`.
    #[must_use]
    pub fn has_base_named(&self, names: &[&str]) -> bool {
        self.bases
            .iter()
            .any(|base| names.contains(&base.last_segment()))
    }

    /// Whether the class statement passed a keyword argument named `keyword`.
    #[must_use]
    pub fn has_class_keyword(&self, keyword: &str) -> bool {
        self.class_keywords.iter().any(|k| k.as_str() == keyword)
    }
}
