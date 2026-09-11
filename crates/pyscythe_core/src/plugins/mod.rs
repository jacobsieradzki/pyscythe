//! Built-in framework plugins, each a set of [`KeepRule`]s.
//!
//! Rules match by convention (decorator names, file paths, symbol names) and
//! never by resolving types, so they stay cheap and predictable. They are
//! deliberately narrow: a rule that keeps too much hides real dead code.

use crate::keep::KeepRule;

mod airflow;
mod alembic;
mod celery;
mod click;
mod django;
mod entry_points;
mod fastapi;
mod flask;
mod pydantic;
mod pytest;
mod python;
mod sqlalchemy;

/// Every built-in plugin, in the order their rules are consulted.
#[must_use]
pub fn all() -> Vec<Box<dyn KeepRule>> {
    vec![
        Box::new(python::Python),
        Box::new(entry_points::EntryPoints),
        Box::new(pytest::Pytest),
        Box::new(fastapi::FastApi),
        Box::new(flask::Flask),
        Box::new(click::Click),
        Box::new(celery::Celery),
        Box::new(airflow::Airflow),
        Box::new(django::Django),
        Box::new(alembic::Alembic),
        Box::new(sqlalchemy::SqlAlchemy),
        Box::new(pydantic::Pydantic),
    ]
}

/// Whether the symbol has a decorator whose last segment is one of `names`
/// and, when ty resolved it, that comes from one of `packages`.
///
/// When `require_receiver` is set the decorator must be an attribute access
/// such as `router.get`, which rules out a bare function called `get`.
/// Unresolved decorators still match by name, so a project without its
/// dependencies installed keeps working. An empty `packages` list accepts any
/// origin.
pub(crate) fn decorated_with_from(
    symbol: &crate::symbol::Symbol,
    names: &[&str],
    require_receiver: bool,
    packages: &[&str],
) -> bool {
    symbol.has_decorator(|decorator| {
        (!require_receiver || decorator.name.has_receiver())
            && names.contains(&decorator.name.last_segment())
            && (packages.is_empty() || decorator.comes_from_any(packages))
    })
}

#[cfg(test)]
pub(crate) mod testing {
    use camino::Utf8PathBuf;

    use crate::index::{Ancestry, SubclassRegistration};
    use crate::keep::{FileRole, KeepContext, KeepRule};
    use crate::manifest::Manifest;
    use crate::source::{ByteOffset, ByteSpan, FileId, MainGuard, ModulePath, SourceFile};
    use crate::symbol::{
        Decorator, DottedName, KeywordName, Symbol, SymbolId, SymbolKind, SymbolName, SymbolScope,
    };

    pub(crate) struct Case {
        pub(crate) path: &'static str,
        pub(crate) module: Option<&'static str>,
        pub(crate) name: &'static str,
        pub(crate) kind: SymbolKind,
        pub(crate) nested: bool,
        pub(crate) decorators: Vec<Decorator>,
        pub(crate) bases: Vec<DottedName>,
        pub(crate) class_keywords: Vec<KeywordName>,
        pub(crate) ancestry: Ancestry,
        pub(crate) manifest: Manifest,
        pub(crate) file_role: FileRole,
        pub(crate) registration: SubclassRegistration,
    }

    impl Case {
        pub(crate) fn function(name: &'static str) -> Self {
            Self {
                path: "/proj/pkg/mod.py",
                module: Some("pkg.mod"),
                name,
                kind: SymbolKind::Function,
                nested: false,
                decorators: Vec::new(),
                bases: Vec::new(),
                class_keywords: Vec::new(),
                ancestry: Ancestry::unknown(),
                manifest: Manifest::empty(),
                file_role: FileRole::Regular,
                registration: SubclassRegistration::NotRegistered,
            }
        }

        pub(crate) fn in_django_settings(mut self) -> Self {
            self.file_role = FileRole::DjangoSettings;
            self
        }

        pub(crate) fn in_tool_config(mut self) -> Self {
            self.file_role = FileRole::ToolConfig;
            self
        }

        pub(crate) fn registered_by_base(mut self) -> Self {
            self.registration = SubclassRegistration::ByBaseHook;
            self
        }

        pub(crate) fn class(name: &'static str) -> Self {
            Self {
                kind: SymbolKind::Class,
                ..Self::function(name)
            }
        }

        /// A method on a class in the same file.
        pub(crate) fn method(name: &'static str) -> Self {
            Self {
                kind: SymbolKind::Method,
                nested: true,
                ..Self::function(name)
            }
        }

        pub(crate) fn variable(name: &'static str) -> Self {
            Self {
                kind: SymbolKind::Variable,
                ..Self::function(name)
            }
        }

        pub(crate) fn at(mut self, path: &'static str) -> Self {
            self.path = path;
            self
        }

        pub(crate) fn in_module(mut self, module: &'static str) -> Self {
            self.module = Some(module);
            self
        }

        pub(crate) fn decorated(mut self, name: &str) -> Self {
            self.decorators.push(Decorator::named(name));
            self
        }

        /// A decorator ty resolved to a definition in `module`.
        pub(crate) fn decorated_from(mut self, name: &str, module: &str) -> Self {
            self.decorators
                .push(Decorator::named(name).from_module(module));
            self
        }

        pub(crate) fn decorated_with_keywords(mut self, name: &str, keywords: &[&str]) -> Self {
            self.decorators.push(Decorator {
                name: DottedName::new(name),
                keywords: keywords.iter().map(|k| KeywordName::new(*k)).collect(),
                module: None,
            });
            self
        }

        pub(crate) fn extending(mut self, base: &str) -> Self {
            self.bases.push(DottedName::new(base));
            self
        }

        /// Every base resolved, to these qualified names.
        pub(crate) fn with_ancestors(mut self, names: &[&str]) -> Self {
            self.ancestry = Ancestry::Complete(names.iter().map(|n| DottedName::new(*n)).collect());
            self
        }

        pub(crate) fn with_class_keyword(mut self, keyword: &str) -> Self {
            self.class_keywords.push(KeywordName::new(keyword));
            self
        }

        pub(crate) fn with_manifest(mut self, manifest: Manifest) -> Self {
            self.manifest = manifest;
            self
        }

        pub(crate) fn keep_reason(&self, rule: &dyn KeepRule) -> Option<&'static str> {
            let file = SourceFile {
                id: FileId::new(0),
                path: Utf8PathBuf::from(self.path),
                relative_path: Utf8PathBuf::from(self.path).components().skip(2).collect(),
                module: self.module.map(ModulePath::new),
                main_guard: MainGuard::Absent,
                exports: Vec::new(),
            };
            let symbol = Symbol {
                id: SymbolId::new(file.id, 0),
                file: file.id,
                name: SymbolName::new(self.name),
                kind: self.kind,
                scope: if self.nested {
                    SymbolScope::Nested {
                        parent: SymbolId::new(file.id, 0),
                    }
                } else {
                    SymbolScope::Module
                },
                decorators: self.decorators.clone(),
                bases: self.bases.clone(),
                class_keywords: self.class_keywords.clone(),
                name_span: ByteSpan::new(ByteOffset::new(4), ByteOffset::new(8)),
                full_span: ByteSpan::new(ByteOffset::new(0), ByteOffset::new(40)),
            };
            rule.keep(KeepContext {
                symbol: &symbol,
                file: &file,
                manifest: &self.manifest,
                ancestry: &self.ancestry,
                public_modules: &[],
                file_role: self.file_role,
                registration: self.registration,
            })
        }

        pub(crate) fn is_kept_by(&self, rule: &dyn KeepRule) -> bool {
            self.keep_reason(rule).is_some()
        }
    }
}
