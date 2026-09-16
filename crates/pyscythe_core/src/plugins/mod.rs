//! Built-in framework plugins, each a set of [`KeepRule`]s.
//!
//! Rules match by convention (decorator names, file paths, symbol names) and
//! never by resolving types, so they stay cheap and predictable. They are
//! deliberately narrow: a rule that keeps too much hides real dead code.

use crate::keep::KeepRule;

mod airflow;
mod alembic;
mod ansible;
mod celery;
mod click;
mod django;
mod entry_points;
mod fastapi;
mod flask;
mod graphene;
mod homeassistant;
mod pydantic;
mod pytest;
mod python;
mod sqlalchemy;
mod textual;

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
        Box::new(graphene::Graphene),
        Box::new(textual::Textual),
        Box::new(homeassistant::HomeAssistant),
        Box::new(ansible::Ansible),
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

    use crate::config::TestCollection;
    use crate::index::{Ancestry, GlobalsAccess, NameUsage, SubclassRegistration};
    use crate::keep::{FileRole, KeepContext, KeepRule};
    use crate::manifest::Manifest;
    use crate::source::{ByteOffset, ByteSpan, FileId, MainGuard, ModulePath, SourceFile};
    use crate::symbol::{
        Decorator, DecoratorCall, DottedName, KeywordName, Provenance, Symbol, SymbolId,
        SymbolKind, SymbolName, SymbolScope,
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
        pub(crate) tests: TestCollection,
        pub(crate) requested_as_parameter: NameUsage,
        pub(crate) globals_access: GlobalsAccess,
        pub(crate) owner: Option<Box<Self>>,
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
                tests: TestCollection::default(),
                requested_as_parameter: NameUsage::Unused,
                globals_access: GlobalsAccess::NotEnumerated,
                owner: None,
            }
        }

        /// pytest configured with these `python_files`, `python_classes`, `python_functions`.
        pub(crate) fn collecting(
            mut self,
            files: &[&str],
            classes: &[&str],
            functions: &[&str],
        ) -> Self {
            self.tests = TestCollection::parse(
                files.iter().copied(),
                classes.iter().copied(),
                functions.iter().copied(),
            )
            .expect("valid patterns");
            self
        }

        pub(crate) fn reading_its_own_globals(mut self) -> Self {
            self.globals_access = GlobalsAccess::Enumerated;
            self
        }

        pub(crate) fn requested_as_parameter(mut self) -> Self {
            self.requested_as_parameter = NameUsage::Used;
            self
        }

        pub(crate) fn in_django_settings(mut self) -> Self {
            self.file_role = FileRole::DjangoSettings;
            self
        }

        pub(crate) fn in_tool_config(mut self) -> Self {
            self.file_role = FileRole::ToolConfig;
            self
        }

        pub(crate) fn in_alembic_script(mut self) -> Self {
            self.file_role = FileRole::AlembicScript;
            self
        }

        pub(crate) fn in_role(mut self, role: FileRole) -> Self {
            self.file_role = role;
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

        /// An attribute assigned in the body of a plain class.
        pub(crate) fn attribute(name: &'static str) -> Self {
            Self {
                kind: SymbolKind::Field,
                nested: true,
                owner: Some(Box::new(Self::class("Owner"))),
                ..Self::function(name)
            }
        }

        /// A class written inside another class.
        pub(crate) fn nested_in_a_class(mut self) -> Self {
            self.nested = true;
            self
        }

        /// The same member, on `owner` instead of a plain class.
        pub(crate) fn on_class(mut self, owner: Self) -> Self {
            self.ancestry = owner.ancestry.clone();
            self.owner = Some(Box::new(owner));
            self
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
                provenance: Provenance::Unknown,
                call: DecoratorCall::Called,
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

        /// Bases ty could not resolve, so only their written names are known.
        pub(crate) fn with_unresolved_ancestors(mut self, names: &[&str]) -> Self {
            self.ancestry =
                Ancestry::Incomplete(names.iter().map(|n| DottedName::new(*n)).collect());
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

        fn symbol(&self, file: FileId, ordinal: u32) -> Symbol {
            Symbol {
                id: SymbolId::new(file, ordinal),
                file,
                name: SymbolName::new(self.name),
                kind: self.kind,
                scope: if self.nested {
                    SymbolScope::Nested {
                        parent: SymbolId::new(file, 0),
                    }
                } else {
                    SymbolScope::Module
                },
                decorators: self.decorators.clone(),
                bases: self.bases.clone(),
                class_keywords: self.class_keywords.clone(),
                name_span: ByteSpan::new(ByteOffset::new(4), ByteOffset::new(8)),
                full_span: ByteSpan::new(ByteOffset::new(0), ByteOffset::new(40)),
            }
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
            let symbol = self.symbol(file.id, 1);
            let owner = self.owner.as_ref().map(|owner| owner.symbol(file.id, 0));
            rule.keep(KeepContext {
                symbol: &symbol,
                owner: owner.as_ref(),
                file: &file,
                manifest: &self.manifest,
                ancestry: &self.ancestry,
                public_modules: &[],
                file_role: self.file_role,
                registration: self.registration,
                tests: &self.tests,
                requested_as_parameter: self.requested_as_parameter,
                globals_access: self.globals_access,
            })
        }

        pub(crate) fn is_kept_by(&self, rule: &dyn KeepRule) -> bool {
            self.keep_reason(rule).is_some()
        }
    }
}
