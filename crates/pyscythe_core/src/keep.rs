//! Rules that stop a symbol being reported even though nothing refers to it.
//!
//! Frameworks reach definitions by convention: a route decorator registers a
//! handler, pytest collects `test_*` functions, an installer imports an entry
//! point. Each convention is a [`KeepRule`]; a [`Policy`] is the set in force.

use serde::Serialize;

use crate::config::{ModulePrefix, TestCollection};
use crate::index::{Ancestry, NameUsage, SubclassRegistration};
use crate::manifest::Manifest;
use crate::source::SourceFile;
use crate::symbol::Symbol;

/// Which plugin a keep rule belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginName {
    /// Conventions of the language itself, such as `@overload`.
    Python,
    /// `[project.scripts]` and `[project.entry-points]`.
    EntryPoints,
    /// pytest collection and fixtures.
    Pytest,
    /// `FastAPI` routers and lifecycle hooks.
    FastApi,
    /// Flask routes and hooks.
    Flask,
    /// Click and Typer commands.
    Click,
    /// Celery tasks and signals.
    Celery,
    /// Airflow DAG and task decorators and DAG folders.
    Airflow,
    /// Django conventions: migrations, commands, models, admin, settings.
    Django,
    /// Alembic migration scripts.
    Alembic,
    /// `SQLAlchemy` event listeners.
    SqlAlchemy,
    /// Pydantic validators and serializers.
    Pydantic,
}

impl PluginName {
    /// The plugin's identifier in output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::EntryPoints => "entry-points",
            Self::Pytest => "pytest",
            Self::FastApi => "fastapi",
            Self::Flask => "flask",
            Self::Click => "click",
            Self::Celery => "celery",
            Self::Airflow => "airflow",
            Self::Django => "django",
            Self::Alembic => "alembic",
            Self::SqlAlchemy => "sqlalchemy",
            Self::Pydantic => "pydantic",
        }
    }
}

/// Why a symbol was kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepReason {
    /// The plugin whose rule fired.
    pub plugin: PluginName,
    /// A short explanation for humans.
    pub why: &'static str,
}

/// What kind of file a symbol lives in, when it changes the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRole {
    /// An ordinary module.
    Regular,
    /// A Django settings module, whose upper-case names are read by the framework.
    DjangoSettings,
    /// A tool's configuration script (`gunicorn.conf.py`, `PyInstaller`
    /// `hook-*.py`), whose module-level names the tool reads.
    ToolConfig,
    /// An Alembic `env.py` or a migration under `versions/`, which Alembic
    /// loads by path.
    AlembicScript,
}

/// Everything a rule may look at when deciding.
#[derive(Debug, Clone, Copy)]
pub struct KeepContext<'a> {
    /// The symbol under consideration.
    pub symbol: &'a Symbol,
    /// The file it lives in.
    pub file: &'a SourceFile,
    /// The project manifest.
    pub manifest: &'a Manifest,
    /// Resolved base classes, for class symbols.
    pub ancestry: &'a Ancestry,
    /// Modules whose public names are API.
    pub public_modules: &'a [ModulePrefix],
    /// What kind of file the symbol lives in.
    pub file_role: FileRole,
    /// Whether a base class registers subclasses on creation.
    pub registration: SubclassRegistration,
    /// How pytest collects tests in this project.
    pub tests: &'a TestCollection,
    /// Whether some function parameter anywhere shares the symbol's name,
    /// the way tests request fixtures.
    pub requested_as_parameter: NameUsage,
}

impl KeepContext<'_> {
    /// Whether the class descends from something in `package`, or, when its
    /// bases could not all be resolved, is written with a base whose last
    /// segment is one of `base_names`.
    #[must_use]
    pub fn descends_from(&self, package: &str, base_names: &[&str]) -> bool {
        self.ancestry.has_ancestor_in(package)
            || (!self.ancestry.is_complete() && self.symbol.has_base_named(base_names))
    }

    /// The file's name without directories, or empty when it has none.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.file.file_name()
    }

    /// Whether pytest would collect this file, by its configured `python_files`.
    #[must_use]
    pub fn is_test_file(&self) -> bool {
        self.tests.is_test_file(self.file_name())
    }

    /// Whether any directory on the file's project-relative path is named `name`.
    #[must_use]
    pub fn is_under_directory(&self, name: &str) -> bool {
        self.file
            .relative_path
            .parent()
            .is_some_and(|dir| dir.components().any(|c| c.as_str() == name))
    }

    /// Whether any directory on the file's path starts with `prefix`, such as
    /// `docs` or `docs_src` for `doc`.
    #[must_use]
    pub fn is_under_directory_starting_with(&self, prefix: &str) -> bool {
        self.file
            .relative_path
            .parent()
            .is_some_and(|dir| dir.components().any(|c| c.as_str().starts_with(prefix)))
    }

    /// Whether the immediate parent directory is named `name`.
    #[must_use]
    pub fn parent_directory_is(&self, name: &str) -> bool {
        self.file
            .relative_path
            .parent()
            .and_then(|dir| dir.file_name())
            .is_some_and(|dir| dir == name)
    }
}

/// One framework convention.
pub trait KeepRule: Send + Sync {
    /// The plugin this rule belongs to.
    fn plugin(&self) -> PluginName;

    /// The reason to keep the symbol, or `None` when this rule does not apply.
    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str>;
}

/// The keep rules in force for one run.
pub struct Policy {
    rules: Vec<Box<dyn KeepRule>>,
}

impl std::fmt::Debug for Policy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let plugins: Vec<_> = self.rules.iter().map(|r| r.plugin().as_str()).collect();
        f.debug_struct("Policy").field("rules", &plugins).finish()
    }
}

impl Policy {
    /// A policy with no rules: every unreferenced symbol is reported.
    #[must_use]
    pub const fn none() -> Self {
        Self { rules: Vec::new() }
    }

    /// A policy from an explicit list of rules.
    #[must_use]
    pub fn from_rules(rules: Vec<Box<dyn KeepRule>>) -> Self {
        Self { rules }
    }

    /// Every built-in framework plugin.
    #[must_use]
    pub fn builtin() -> Self {
        Self::from_rules(crate::plugins::all())
    }

    /// The first rule that keeps the symbol, if any.
    #[must_use]
    pub fn keep_reason(&self, context: KeepContext<'_>) -> Option<KeepReason> {
        self.rules.iter().find_map(|rule| {
            rule.keep(context).map(|why| KeepReason {
                plugin: rule.plugin(),
                why,
            })
        })
    }
}
