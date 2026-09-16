//! Turns `pyproject.toml` into a [`Manifest`] and a [`Config`].

use std::collections::{BTreeMap, BTreeSet};

use camino::Utf8Path;
use pyscythe_core::config::{
    BoundaryConfig, Config, DenyRule, HealthThresholds, ModulePrefix, NamePatterns, NotebookPolicy,
    PathPatterns, PatternError, TestCollection, TypeOnlyImports,
};
use pyscythe_core::deps::ManifestScope;
use pyscythe_core::manifest::{
    Dependency, DependencyGroup, DistributionName, EntryPoint, EntryPointKind, Manifest,
};
use pyscythe_core::source::ModulePath;
use pyscythe_core::symbol::SymbolName;
use serde::Deserialize;

mod pytest_config;

pub use pytest_config::PyprojectPytest;

/// What `pyproject.toml` told us.
#[derive(Debug, Clone)]
pub struct ProjectSettings {
    /// Whether the root `pyproject.toml` configured pytest, which decides
    /// whether `tox.ini` and `setup.cfg` are consulted.
    pub pytest: PyprojectPytest,
    /// Every entry point and dependency across the project, merged.
    pub manifest: Manifest,
    /// User configuration from the root `[tool.pyscythe]`.
    pub config: Config,
    /// One manifest per directory that declares one, root first.
    pub scopes: Vec<ManifestScope>,
    /// The project's distribution name from `[project]` or `[tool.poetry]`.
    pub name: Option<DistributionName>,
}

/// Why a manifest could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// The file exists but could not be read.
    #[error("could not read {path}: {source}")]
    Io {
        /// The file that failed.
        path: camino::Utf8PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The file is not valid TOML or has the wrong shape.
    #[error("could not parse {path}: {source}")]
    Parse {
        /// The file that failed.
        path: camino::Utf8PathBuf,
        /// The underlying error.
        #[source]
        source: ParseError,
    },
}

/// Why `pyproject.toml` text was rejected.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// Malformed TOML or an unexpected shape.
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
    /// A glob in `[tool.pyscythe]` did not parse.
    #[error(transparent)]
    Pattern(#[from] PatternError),
    /// `[tool.pyscythe.boundaries]` is inconsistent.
    #[error("invalid [tool.pyscythe.boundaries]: {0}")]
    Boundaries(String),
}

fn boundary_config(table: BoundariesTable) -> Result<BoundaryConfig, ParseError> {
    let mut config = match table.preset.as_deref() {
        None => BoundaryConfig {
            layers: Vec::new(),
            rules: Vec::new(),
            type_only: TypeOnlyImports::Allow,
        },
        Some("hexagonal") => {
            let root = table.root.as_deref().ok_or_else(|| {
                ParseError::Boundaries(
                    "preset \"hexagonal\" needs `root`, the package it applies to".into(),
                )
            })?;
            BoundaryConfig::hexagonal(root)
        }
        Some(other) => {
            return Err(ParseError::Boundaries(format!(
                "unknown preset \"{other}\"; only \"hexagonal\" is built in"
            )));
        }
    };
    config
        .layers
        .extend(table.layers.into_iter().map(|rank| match rank {
            LayerEntry::One(name) => vec![ModulePrefix::new(name)],
            LayerEntry::Many(names) => names.into_iter().map(ModulePrefix::new).collect(),
        }));
    config
        .rules
        .extend(table.rules.into_iter().map(|rule| DenyRule {
            from: ModulePrefix::new(rule.from),
            deny: rule.deny.into_iter().map(ModulePrefix::new).collect(),
            allow: rule.allow.into_iter().map(ModulePrefix::new).collect(),
        }));
    if table.check_type_only {
        config.type_only = TypeOnlyImports::Check;
    }
    if config.is_empty() {
        return Err(ParseError::Boundaries(
            "configure `layers`, `rules`, or a `preset`".into(),
        ));
    }
    Ok(config)
}

/// Loads settings for the project rooted at `root`.
///
/// The root `pyproject.toml` supplies configuration, and every directory
/// beneath that holds a `pyproject.toml`, `setup.py`, `setup.cfg`, or
/// `requirements.txt` (environments and build output skipped) supplies a
/// manifest, so nested projects are judged against their own declarations.
///
/// # Errors
///
/// Returns [`ManifestError`] when a file exists but cannot be read or parsed.
pub fn load(root: &Utf8Path) -> Result<ProjectSettings, ManifestError> {
    load_with_config(root, None)
}

/// The same, with `[tool.pyscythe]` read from `config_path` instead of from
/// the project's own `pyproject.toml`.
///
/// The manifest still comes from the project: what it declares about itself is
/// a fact about the project, while how to analyse it is the caller's to choose.
///
/// # Errors
///
/// Returns [`ManifestError`] when a file exists but cannot be read or parsed.
pub fn load_with_config(
    root: &Utf8Path,
    config_path: Option<&Utf8Path>,
) -> Result<ProjectSettings, ManifestError> {
    let mut scopes = Vec::new();
    let mut root_settings = RootSettings::default();
    let mut directories = vec![root.to_path_buf()];
    directories.extend(manifest_directories(root));
    for directory in directories {
        let Some(scope) = manifest_in(&directory, &mut root_settings, directory.as_path() == root)?
        else {
            continue;
        };
        scopes.push(scope);
    }
    if let Some(path) = config_path {
        let text = std::fs::read_to_string(path).map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let settings = parse(&text).map_err(|source| ManifestError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        root_settings.config = settings.config;
        root_settings.pytest = settings.pytest;
    }
    let RootSettings { mut config, pytest } = root_settings;
    config.tests =
        pytest_config::resolve(root, pytest, config.tests.clone()).map_err(|source| {
            ManifestError::Parse {
                path: root.join("pytest.ini"),
                source: ParseError::Pattern(source),
            }
        })?;
    let name = scopes.first().and_then(|scope| scope.name.clone());
    Ok(ProjectSettings {
        manifest: Manifest::merged(scopes.iter().map(|s| &s.manifest)),
        config,
        scopes,
        pytest,
        name,
    })
}

/// The manifest declared in `directory` and the file it came from: from
/// `pyproject.toml` first, then `setup.py` / `setup.cfg` for dependencies
/// when the project table has none.
/// `[tool.pyscythe.health]` over the defaults.
fn health_thresholds(table: &HealthTable) -> HealthThresholds {
    let defaults = HealthThresholds::default();
    HealthThresholds {
        max_cyclomatic: table.cyclomatic.unwrap_or(defaults.max_cyclomatic),
        max_cognitive: table.cognitive.unwrap_or(defaults.max_cognitive),
        max_lines: table.lines.unwrap_or(defaults.max_lines),
        max_parameters: table.parameters.unwrap_or(defaults.max_parameters),
    }
}

/// The import name a distribution conventionally installs as: `Django` is
/// `django`, `langchain-classic` is `langchain_classic`.
fn package_name_of(distribution: &str) -> String {
    distribution.trim().to_lowercase().replace(['-', '.'], "_")
}

/// What the root `pyproject.toml` contributes besides its manifest.
#[derive(Debug, Default)]
struct RootSettings {
    config: Config,
    pytest: PyprojectPytest,
}

fn manifest_in(
    directory: &Utf8Path,
    root_settings: &mut RootSettings,
    is_root: bool,
) -> Result<Option<ManifestScope>, ManifestError> {
    let pyproject = directory.join("pyproject.toml");
    let mut source_path = pyproject.clone();
    let mut name = None;
    let mut manifest = match std::fs::read_to_string(&pyproject) {
        Ok(text) => {
            let settings = parse(&text).map_err(|source| ManifestError::Parse {
                path: pyproject,
                source,
            })?;
            if is_root {
                root_settings.config = settings.config;
                root_settings.pytest = settings.pytest;
            } else {
                // A nested project's package is a library package too.
                root_settings
                    .config
                    .library_packages
                    .extend(settings.config.library_packages);
            }
            name = settings.name;
            Some(settings.manifest)
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(ManifestError::Io {
                path: pyproject,
                source,
            });
        }
    };
    if manifest.as_ref().is_none_or(|m| m.dependencies.is_empty()) {
        for (file_name, read) in [
            (
                "setup.py",
                setup_py_dependencies as fn(&str) -> Vec<Dependency>,
            ),
            ("setup.cfg", setup_cfg_dependencies),
        ] {
            let path = directory.join(file_name);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let legacy = read(&text);
            if legacy.is_empty() {
                continue;
            }
            source_path = path;
            manifest
                .get_or_insert_with(Manifest::empty)
                .dependencies
                .extend(legacy);
            break;
        }
    }
    // Entry points are additive: a setuptools project declares its scripts in
    // `setup.py` or `setup.cfg` whether or not a `pyproject.toml` sits beside
    // it, and each one keeps a module alive.
    for (file_name, read) in [
        (
            "setup.py",
            setup_py_entry_points as fn(&str) -> Vec<EntryPoint>,
        ),
        ("setup.cfg", setup_cfg_entry_points),
    ] {
        let path = directory.join(file_name);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let declared = read(&text);
        if declared.is_empty() {
            continue;
        }
        if manifest.is_none() {
            source_path = path;
        }
        manifest
            .get_or_insert_with(Manifest::empty)
            .entry_points
            .extend(declared);
    }
    if manifest.as_ref().is_none_or(|m| m.dependencies.is_empty())
        && let Some((path, pinned)) = requirements_dependencies(directory)
    {
        source_path = path;
        manifest
            .get_or_insert_with(Manifest::empty)
            .dependencies
            .extend(pinned);
    }
    Ok(manifest.map(|manifest| ManifestScope {
        manifest_path: source_path,
        manifest,
        name,
    }))
}

/// Dependencies from pip requirements files: `requirements.txt`,
/// `requirements-*.txt`, `requirements_*.txt`, and `requirements/*.txt`,
/// following `-r` includes. Returns the entry file (`requirements.txt` when
/// there is one) alongside them.
fn requirements_dependencies(
    directory: &Utf8Path,
) -> Option<(camino::Utf8PathBuf, Vec<Dependency>)> {
    let mut files: Vec<camino::Utf8PathBuf> = Vec::new();
    if let Ok(entries) = directory.read_dir_utf8() {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default();
            let variant = (name.starts_with("requirements-") || name.starts_with("requirements_"))
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"));
            if name == "requirements.txt" || variant {
                files.push(path.to_path_buf());
            }
        }
    }
    if let Ok(entries) = directory.join("requirements").read_dir_utf8() {
        files.extend(
            entries
                .flatten()
                .map(|entry| entry.path().to_path_buf())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
                }),
        );
    }
    files.sort();
    if let Some(index) = files
        .iter()
        .position(|path| path.file_name() == Some("requirements.txt"))
    {
        files.swap(0, index);
    }
    let entry = files.first()?.clone();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for file in &files {
        read_requirements(file, &mut seen, &mut out);
    }
    Some((entry, out))
}

fn read_requirements(
    path: &Utf8Path,
    seen: &mut BTreeSet<camino::Utf8PathBuf>,
    out: &mut Vec<Dependency>,
) {
    if !seen.insert(path.to_path_buf()) {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let group = requirements_group(path);
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if let Some(include) = line
            .strip_prefix("-r ")
            .or_else(|| line.strip_prefix("--requirement "))
        {
            let included = path.parent().map_or_else(
                || camino::Utf8PathBuf::from(include.trim()),
                |dir| dir.join(include.trim()),
            );
            read_requirements(&included, seen, out);
            continue;
        }
        // Options, editable installs, and bare URLs name no distribution.
        if line.starts_with('-') || line.starts_with("git+") || line.starts_with("http") {
            continue;
        }
        if let Some(name) = requirement_name(line)
            && !out.iter().any(|d| d.name == name && d.group == group)
        {
            out.push(Dependency {
                name,
                group: group.clone(),
            });
        }
    }
}

/// `requirements.txt` and `base.txt` are the runtime set; any other file is a
/// group named after it (`requirements-dev.txt` is `dev`).
fn requirements_group(path: &Utf8Path) -> DependencyGroup {
    match path.file_stem().unwrap_or_default() {
        "requirements" | "base" | "main" | "prod" | "production" => DependencyGroup::Main,
        stem => DependencyGroup::Group(
            stem.strip_prefix("requirements")
                .map_or(stem, |rest| rest.trim_start_matches(['-', '_']))
                .to_owned(),
        ),
    }
}

/// Directories under `root` (excluding it) that carry a manifest file.
fn manifest_directories(root: &Utf8Path) -> Vec<camino::Utf8PathBuf> {
    const SKIP: &[&str] = &[
        ".venv",
        "venv",
        "node_modules",
        "target",
        "build",
        "dist",
        "site-packages",
        "__pycache__",
    ];
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = dir.read_dir_utf8() else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                // Projects under a tests tree are fixtures for the tests, not
                // part of the code base.
                let is_test_tree = matches!(name, "tests" | "test" | "fixtures" | "testdata");
                if !name.starts_with('.') && !SKIP.contains(&name) && !is_test_tree {
                    pending.push(path.to_path_buf());
                }
            } else if matches!(
                name,
                "pyproject.toml" | "setup.py" | "setup.cfg" | "requirements.txt"
            ) && dir != root
                && !found.contains(&dir)
            {
                found.push(dir.clone());
            }
        }
    }
    found.sort();
    found
}

/// `install_requires` and `extras_require` string literals in a `setup()` call.
fn setup_py_dependencies(source: &str) -> Vec<Dependency> {
    use ruff_python_ast::{Expr, Stmt};
    let Ok(parsed) = ruff_python_parser::parse_module(source) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let strings = |value: &Expr| -> Vec<String> {
        match value {
            Expr::List(list) => list
                .elts
                .iter()
                .filter_map(|e| match e {
                    Expr::StringLiteral(s) => Some(s.value.to_str().to_owned()),
                    _ => None,
                })
                .collect(),
            Expr::Tuple(tuple) => tuple
                .elts
                .iter()
                .filter_map(|e| match e {
                    Expr::StringLiteral(s) => Some(s.value.to_str().to_owned()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    };
    for statement in &parsed.syntax().body {
        let Stmt::Expr(expression) = statement else {
            continue;
        };
        let Expr::Call(call) = &*expression.value else {
            continue;
        };
        if !is_setup_call(&call.func) {
            continue;
        }
        for keyword in &call.arguments.keywords {
            match keyword.arg.as_deref() {
                Some("install_requires") => out.extend(
                    strings(&keyword.value)
                        .iter()
                        .filter_map(|r| requirement_name(r))
                        .map(|name| Dependency {
                            name,
                            group: DependencyGroup::Main,
                        }),
                ),
                Some("extras_require") => {
                    if let Expr::Dict(dict) = &keyword.value {
                        for item in &dict.items {
                            let group = match &item.key {
                                Some(Expr::StringLiteral(s)) => s.value.to_str().to_owned(),
                                _ => continue,
                            };
                            out.extend(
                                strings(&item.value)
                                    .iter()
                                    .filter_map(|r| requirement_name(r))
                                    .map(|name| Dependency {
                                        name,
                                        group: DependencyGroup::Optional(group.clone()),
                                    }),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// The entry points a `setup()` call declares, from the `entry_points=`
/// keyword. setuptools takes either a dict of group name to a list of
/// `name = module:attr` strings, or one INI-format string holding the same
/// thing; both are read here.
fn setup_py_entry_points(source: &str) -> Vec<EntryPoint> {
    use ruff_python_ast::{Expr, Stmt};
    let Ok(parsed) = ruff_python_parser::parse_module(source) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for statement in &parsed.syntax().body {
        let Stmt::Expr(expression) = statement else {
            continue;
        };
        let Expr::Call(call) = &*expression.value else {
            continue;
        };
        if !is_setup_call(&call.func) {
            continue;
        }
        for keyword in &call.arguments.keywords {
            if keyword.arg.as_deref() != Some("entry_points") {
                continue;
            }
            match &keyword.value {
                Expr::Dict(dict) => {
                    for item in &dict.items {
                        let Some(Expr::StringLiteral(group)) = &item.key else {
                            continue;
                        };
                        let kind = group_kind(group.value.to_str());
                        out.extend(
                            string_elements(&item.value)
                                .iter()
                                .filter_map(|line| declared_entry_point(kind, line)),
                        );
                    }
                }
                Expr::StringLiteral(text) => {
                    out.extend(ini_entry_points(text.value.to_str()));
                }
                _ => {}
            }
        }
    }
    out
}

/// Whether the called expression is `setup` or `setuptools.setup`.
fn is_setup_call(func: &ruff_python_ast::Expr) -> bool {
    use ruff_python_ast::Expr;
    match func {
        Expr::Name(name) => name.id.as_str() == "setup",
        Expr::Attribute(attribute) => attribute.attr.as_str() == "setup",
        _ => false,
    }
}

/// The string literals of a list or tuple expression.
fn string_elements(value: &ruff_python_ast::Expr) -> Vec<String> {
    use ruff_python_ast::Expr;
    let elements = match value {
        Expr::List(list) => &list.elts,
        Expr::Tuple(tuple) => &tuple.elts,
        _ => return Vec::new(),
    };
    elements
        .iter()
        .filter_map(|element| match element {
            Expr::StringLiteral(s) => Some(s.value.to_str().to_owned()),
            _ => None,
        })
        .collect()
}

/// `[options.entry_points]` in `setup.cfg`: group names at the left margin,
/// each followed by indented `name = module:attr` lines.
fn setup_cfg_entry_points(text: &str) -> Vec<EntryPoint> {
    let mut out = Vec::new();
    let mut in_section = false;
    let mut kind = EntryPointKind::Plugin;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == "[options.entry_points]";
            continue;
        }
        if !in_section || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indented = line.starts_with(char::is_whitespace);
        if indented {
            if let Some(entry) = declared_entry_point(kind, trimmed) {
                out.push(entry);
            }
            continue;
        }
        // `console_scripts =` opens a group; `console_scripts = a = pkg:main`
        // declares one on the same line.
        let Some((group, rest)) = trimmed.split_once('=') else {
            continue;
        };
        kind = group_kind(group.trim());
        if let Some(entry) = declared_entry_point(kind, rest.trim()) {
            out.push(entry);
        }
    }
    out
}

/// The entry points an INI-format `entry_points=` string declares.
fn ini_entry_points(text: &str) -> Vec<EntryPoint> {
    let mut out = Vec::new();
    let mut kind = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(group) = trimmed.strip_prefix('[').and_then(|g| g.strip_suffix(']')) {
            kind = Some(group_kind(group.trim()));
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(kind) = kind
            && let Some(entry) = declared_entry_point(kind, trimmed)
        {
            out.push(entry);
        }
    }
    out
}

/// What kind of entry point a group name declares.
fn group_kind(group: &str) -> EntryPointKind {
    match group {
        "console_scripts" => EntryPointKind::Script,
        "gui_scripts" => EntryPointKind::GuiScript,
        _ => EntryPointKind::Plugin,
    }
}

/// `name = pkg.module:attr`, as written in a group.
fn declared_entry_point(kind: EntryPointKind, line: &str) -> Option<EntryPoint> {
    let (_, target) = line.split_once('=')?;
    let target = target.trim();
    (!target.is_empty()).then(|| entry_point(kind, target))
}

/// `install_requires =` under `[options]` in `setup.cfg`, one requirement per line.
fn setup_cfg_dependencies(text: &str) -> Vec<Dependency> {
    let mut out = Vec::new();
    let mut in_options = false;
    let mut in_requires = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_options = trimmed == "[options]";
            in_requires = false;
            continue;
        }
        if !in_options {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("install_requires") {
            in_requires = true;
            if let Some(inline) = rest.trim_start().strip_prefix('=') {
                out.extend(requirement_name(inline).map(|name| Dependency {
                    name,
                    group: DependencyGroup::Main,
                }));
            }
            continue;
        }
        if in_requires {
            if line.starts_with(char::is_whitespace) && !trimmed.is_empty() {
                out.extend(requirement_name(trimmed).map(|name| Dependency {
                    name,
                    group: DependencyGroup::Main,
                }));
            } else {
                in_requires = false;
            }
        }
    }
    out
}

/// Parses `pyproject.toml` text.
///
/// # Errors
///
/// Returns [`ParseError`] when the text is malformed or a glob is invalid.
pub fn parse(text: &str) -> Result<ProjectSettings, ParseError> {
    let document: PyProject = toml::from_str(text)?;
    let project = document.project.unwrap_or_default();
    let tables = document.tool.unwrap_or_default();
    let tool = tables.pyscythe.unwrap_or_default();
    let poetry = tables.poetry.unwrap_or_default();
    let (tests, pytest) = match tables.pytest.and_then(|table| table.ini_options) {
        Some(options) => (
            pytest_config::from_ini_options(&options)?,
            PyprojectPytest::Configured,
        ),
        None => (TestCollection::default(), PyprojectPytest::Absent),
    };

    let mut entry_points = Vec::new();
    entry_points.extend(targets(&project.scripts, EntryPointKind::Script));
    entry_points.extend(targets(&poetry.scripts, EntryPointKind::Script));
    entry_points.extend(targets(&project.gui_scripts, EntryPointKind::GuiScript));
    for group in project.entry_points.values() {
        entry_points.extend(targets(group, EntryPointKind::Plugin));
    }
    entry_points.extend(
        tool.entry_points
            .iter()
            .map(|target| entry_point(EntryPointKind::Plugin, target)),
    );

    let mut dependencies: Vec<Dependency> = project
        .dependencies
        .iter()
        .filter_map(|requirement| requirement_name(requirement))
        .map(|name| Dependency {
            name,
            group: DependencyGroup::Main,
        })
        .collect();
    for (group, requirements) in &project.optional_dependencies {
        dependencies.extend(
            requirements
                .iter()
                .filter_map(|r| requirement_name(r))
                .map(|name| Dependency {
                    name,
                    group: DependencyGroup::Optional(group.clone()),
                }),
        );
    }
    let groups = document.dependency_groups.unwrap_or_default();
    for group in groups.keys() {
        let mut visited = Vec::new();
        for name in group_requirements(&groups, group, &mut visited) {
            dependencies.push(Dependency {
                name,
                group: DependencyGroup::Group(group.clone()),
            });
        }
    }
    dependencies.extend(poetry.dependencies());

    let config = Config {
        exclude: PathPatterns::parse(tool.exclude.iter().map(String::as_str))?,
        ignore_names: NamePatterns::parse(tool.ignore_names.iter().map(String::as_str))?,
        notebooks: if tool.include_notebooks {
            NotebookPolicy::Include
        } else {
            NotebookPolicy::Exclude
        },
        boundaries: tool.boundaries.map(boundary_config).transpose()?,
        public_modules: tool
            .public_modules
            .iter()
            .map(|name| ModulePrefix::new(name.as_str()))
            .collect(),
        tests,
        library_packages: project
            .name
            .as_deref()
            .or(poetry.name.as_deref())
            .map(|name| ModulePrefix::new(package_name_of(name)))
            .into_iter()
            .collect(),
        ignored_dependencies: tool
            .deps
            .unwrap_or_default()
            .ignore
            .iter()
            .map(|name| DistributionName::normalize(name))
            .collect(),
        health: health_thresholds(&tool.health.unwrap_or_default()),
    };

    Ok(ProjectSettings {
        manifest: Manifest {
            entry_points,
            dependencies,
        },
        config,
        scopes: Vec::new(),
        pytest,
        name: project
            .name
            .as_deref()
            .or(poetry.name.as_deref())
            .map(DistributionName::normalize),
    })
}

/// The requirements of a PEP 735 group, following `include-group` entries.
fn group_requirements(
    groups: &BTreeMap<String, Vec<GroupEntry>>,
    group: &str,
    visited: &mut Vec<String>,
) -> Vec<DistributionName> {
    if visited.iter().any(|seen| seen == group) {
        return Vec::new();
    }
    visited.push(group.to_owned());
    let mut names = Vec::new();
    for entry in groups.get(group).map(Vec::as_slice).unwrap_or_default() {
        match entry {
            GroupEntry::Requirement(requirement) => names.extend(requirement_name(requirement)),
            GroupEntry::Include(include) => {
                names.extend(group_requirements(groups, &include.include_group, visited));
            }
        }
    }
    names
}

/// The project name at the front of a PEP 508 requirement such as
/// `fastapi[standard]>=0.104; python_version < "3.13"`.
fn requirement_name(requirement: &str) -> Option<DistributionName> {
    let name: String = requirement
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    (!name.is_empty()).then(|| DistributionName::normalize(&name))
}

fn targets(
    table: &BTreeMap<String, String>,
    kind: EntryPointKind,
) -> impl Iterator<Item = EntryPoint> + '_ {
    table.values().map(move |target| entry_point(kind, target))
}

/// Splits `module:attr` (or `module:attr.sub`, or a bare module) into an entry point.
fn entry_point(kind: EntryPointKind, target: &str) -> EntryPoint {
    let (module, attribute) = match target.split_once(':') {
        Some((module, attribute)) => (module, Some(attribute)),
        None => (target, None),
    };
    // A dotted attribute such as `Cli.run` is reached through its first segment.
    let attribute = attribute
        .and_then(|a| a.split('.').next())
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(SymbolName::new);
    EntryPoint {
        kind,
        module: ModulePath::new(module.trim()),
        attribute,
    }
}

#[derive(Debug, Deserialize)]
struct PyProject {
    project: Option<Project>,
    tool: Option<Tool>,
    #[serde(rename = "dependency-groups")]
    dependency_groups: Option<BTreeMap<String, Vec<GroupEntry>>>,
}

/// A PEP 735 group entry: a requirement string or an `{include-group = ...}` table.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum GroupEntry {
    Requirement(String),
    Include(IncludeGroup),
}

#[derive(Debug, Deserialize)]
struct IncludeGroup {
    #[serde(rename = "include-group")]
    include_group: String,
}

#[derive(Debug, Default, Deserialize)]
struct Tool {
    pyscythe: Option<PyscytheTable>,
    poetry: Option<PoetryTable>,
    pytest: Option<PytestTool>,
}

/// `[tool.pytest]`; only `ini_options` matters here.
#[derive(Debug, Default, Deserialize)]
struct PytestTool {
    #[serde(default)]
    ini_options: Option<pytest_config::IniOptions>,
}

/// `[tool.poetry]`: dependencies keyed by name (the value is a constraint or
/// a table), optional groups, and scripts.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct PoetryTable {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    dependencies: BTreeMap<String, toml::Value>,
    #[serde(default)]
    dev_dependencies: BTreeMap<String, toml::Value>,
    #[serde(default)]
    group: BTreeMap<String, PoetryGroup>,
    #[serde(default)]
    scripts: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
struct PoetryGroup {
    #[serde(default)]
    dependencies: BTreeMap<String, toml::Value>,
}

impl PoetryTable {
    /// Every declared requirement; `python` is the interpreter constraint, not one.
    fn dependencies(&self) -> Vec<Dependency> {
        let named = |names: &BTreeMap<String, toml::Value>, group: DependencyGroup| {
            names
                .keys()
                .filter(|name| name.as_str() != "python")
                .map(|name| Dependency {
                    name: DistributionName::normalize(name),
                    group: group.clone(),
                })
                .collect::<Vec<_>>()
        };
        let mut out = named(&self.dependencies, DependencyGroup::Main);
        out.extend(named(
            &self.dev_dependencies,
            DependencyGroup::Group("dev".to_owned()),
        ));
        for (group, table) in &self.group {
            out.extend(named(
                &table.dependencies,
                DependencyGroup::Group(group.clone()),
            ));
        }
        out
    }
}

/// `[tool.pyscythe]`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct PyscytheTable {
    /// Project-relative globs for files to leave out of reports.
    #[serde(default)]
    exclude: Vec<String>,
    /// Globs for symbol names never to report.
    #[serde(default)]
    ignore_names: Vec<String>,
    /// Extra `module:attr` or `module` roots.
    #[serde(default)]
    entry_points: Vec<String>,
    /// Analyse notebook cells like modules.
    #[serde(default)]
    include_notebooks: bool,
    /// Modules whose public names are the project's API.
    #[serde(default)]
    public_modules: Vec<String>,
    /// Architecture boundaries.
    boundaries: Option<BoundariesTable>,
    /// Health thresholds.
    health: Option<HealthTable>,
    /// Dependency analysis settings.
    deps: Option<DepsTable>,
}

/// `[tool.pyscythe.deps]`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct DepsTable {
    /// Distributions never reported as unused.
    #[serde(default)]
    ignore: Vec<String>,
}

/// `[tool.pyscythe.health]`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct HealthTable {
    #[serde(rename = "max-cyclomatic")]
    cyclomatic: Option<u32>,
    #[serde(rename = "max-cognitive")]
    cognitive: Option<u32>,
    #[serde(rename = "max-lines")]
    lines: Option<u32>,
    #[serde(rename = "max-parameters")]
    parameters: Option<u32>,
}

/// `[tool.pyscythe.boundaries]`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct BoundariesTable {
    /// A built-in shape: currently `hexagonal`.
    preset: Option<String>,
    /// The package a preset applies to.
    root: Option<String>,
    /// Ranks from top to bottom; each entry is one prefix or several sharing a rank.
    #[serde(default)]
    layers: Vec<LayerEntry>,
    /// Explicit prohibitions.
    #[serde(default)]
    rules: Vec<RuleTable>,
    /// Hold `if TYPE_CHECKING:` imports to the rules too.
    #[serde(default)]
    check_type_only: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum LayerEntry {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct RuleTable {
    from: String,
    deny: Vec<String>,
    #[serde(default)]
    allow: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Project {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    optional_dependencies: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    scripts: BTreeMap<String, String>,
    #[serde(default)]
    gui_scripts: BTreeMap<String, String>,
    #[serde(default)]
    entry_points: BTreeMap<String, BTreeMap<String, String>>,
}

#[cfg(test)]
mod tests {
    use super::parse;
    use pyscythe_core::config::{NotebookPolicy, TypeOnlyImports};
    use pyscythe_core::manifest::{DependencyGroup, EntryPointKind};
    use pyscythe_core::symbol::SymbolName;

    #[test]
    fn reads_poetry_dependencies_groups_and_scripts() {
        let settings = parse(
            r#"
[tool.poetry]
name = "demo"

[tool.poetry.dependencies]
python = "^3.12"
requests = "^2.22.0"
Pillow = { version = "^10", optional = true }

[tool.poetry.group.dev.dependencies]
pytest = "^8"

[tool.poetry.scripts]
demo = "demo.cli:main"
"#,
        )
        .unwrap();

        let pairs: Vec<(&str, DependencyGroup)> = settings
            .manifest
            .dependencies
            .iter()
            .map(|d| (d.name.as_str(), d.group.clone()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("pillow", DependencyGroup::Main),
                ("requests", DependencyGroup::Main),
                ("pytest", DependencyGroup::Group("dev".into())),
            ]
        );
        let modules: Vec<&str> = settings
            .manifest
            .entry_points
            .iter()
            .map(|e| e.module.as_str())
            .collect();
        assert_eq!(modules, ["demo.cli"]);
    }

    #[test]
    fn reads_scripts_gui_scripts_and_plugin_groups() {
        let manifest = parse(
            r#"
[project]
name = "demo"

[project.scripts]
demo = "demo.cli:main"

[project.gui-scripts]
demo-gui = "demo.gui:App.run"

[project.entry-points."pytest11"]
demo = "demo.plugin"
"#,
        )
        .unwrap();

        let summary: Vec<_> = manifest
            .manifest
            .entry_points
            .iter()
            .map(|ep| {
                (
                    ep.kind,
                    ep.module.as_str(),
                    ep.attribute.as_ref().map(SymbolName::as_str),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (EntryPointKind::Script, "demo.cli", Some("main")),
                (EntryPointKind::GuiScript, "demo.gui", Some("App")),
                (EntryPointKind::Plugin, "demo.plugin", None),
            ]
        );
    }

    #[test]
    fn a_project_without_entry_points_is_empty() {
        let settings = parse("[project]\nname = \"demo\"\n").unwrap();
        assert!(settings.manifest.entry_points.is_empty());
    }

    #[test]
    fn a_file_without_a_project_table_is_empty() {
        let settings = parse("[tool.ruff]\nline-length = 100\n").unwrap();
        assert!(settings.manifest.entry_points.is_empty());
    }

    #[test]
    fn malformed_toml_is_an_error() {
        assert!(parse("[project\n").is_err());
    }

    #[test]
    fn reads_the_tool_table() {
        let settings = parse(
            r#"
[tool.pyscythe]
exclude = ["scripts", "**/legacy_*.py"]
ignore-names = ["deprecated_*"]
entry-points = ["pkg.worker:run", "pkg.plugin"]
include-notebooks = true
"#,
        )
        .unwrap();

        assert!(
            settings
                .config
                .exclude
                .matches(camino::Utf8Path::new("scripts/x.py"))
        );
        assert!(settings.config.ignore_names.matches("deprecated_thing"));
        assert_eq!(settings.config.notebooks, NotebookPolicy::Include);
        let roots: Vec<_> = settings
            .manifest
            .entry_points
            .iter()
            .map(|ep| {
                (
                    ep.module.as_str(),
                    ep.attribute.as_ref().map(SymbolName::as_str),
                )
            })
            .collect();
        assert_eq!(roots, [("pkg.worker", Some("run")), ("pkg.plugin", None)]);
    }

    #[test]
    fn reads_boundaries_with_layers_rules_and_presets() {
        let settings = parse(
            r#"
[tool.pyscythe.boundaries]
layers = ["app.api", ["app.services", "app.workers"], "app.domain"]
check-type-only = true

[[tool.pyscythe.boundaries.rules]]
from = "app.domain"
deny = ["app.infra"]
"#,
        )
        .unwrap();
        let boundaries = settings.config.boundaries.expect("boundaries");
        assert_eq!(boundaries.layers.len(), 3);
        assert_eq!(boundaries.rank_of("app.workers.jobs"), Some(1));
        assert_eq!(boundaries.rules.len(), 1);
        assert_eq!(boundaries.type_only, TypeOnlyImports::Check);

        let preset =
            parse("[tool.pyscythe.boundaries]\npreset = \"hexagonal\"\nroot = \"app\"\n").unwrap();
        assert_eq!(
            preset
                .config
                .boundaries
                .expect("preset")
                .rank_of("app.domain.x"),
            Some(2)
        );
    }

    #[test]
    fn reads_dependencies_from_every_table() {
        let settings = parse(
            r#"
[project]
name = "demo"
dependencies = ["fastapi[standard]>=0.104", "Pillow", "python_dateutil ; python_version < '3.13'"]

[project.optional-dependencies]
dev = ["pytest>=8"]

[dependency-groups]
lint = ["ruff", {include-group = "dev"}]

[tool.pyscythe.deps]
ignore = ["my_plugin"]
"#,
        )
        .unwrap();
        let names: Vec<(&str, DependencyGroup)> = settings
            .manifest
            .dependencies
            .iter()
            .map(|d| (d.name.as_str(), d.group.clone()))
            .collect();
        assert_eq!(
            names,
            [
                ("fastapi", DependencyGroup::Main),
                ("pillow", DependencyGroup::Main),
                ("python-dateutil", DependencyGroup::Main),
                ("pytest", DependencyGroup::Optional("dev".into())),
                ("ruff", DependencyGroup::Group("lint".into())),
            ],
            "include-group = \"dev\" refers to an optional-dependencies name, not a group, so it adds nothing"
        );
        assert_eq!(
            settings.config.ignored_dependencies[0].as_str(),
            "my-plugin"
        );
    }

    #[test]
    fn reads_entry_points_from_setup_py_as_a_dict_or_as_one_ini_string() {
        let described = |points: &[super::EntryPoint]| -> Vec<(EntryPointKind, String, String)> {
            points
                .iter()
                .map(|point| {
                    (
                        point.kind,
                        point.module.as_str().to_owned(),
                        point
                            .attribute
                            .as_ref()
                            .map_or_else(String::new, |a| a.as_str().to_owned()),
                    )
                })
                .collect()
        };
        let dict = super::setup_py_entry_points(
            "from setuptools import setup\nsetup(entry_points={'console_scripts': ['legacy = pkg.cli:main'], 'gui_scripts': ('legacy-gui = pkg.gui:launch',), 'pytest11': ['legacy = pkg.plugin']})\n",
        );
        assert_eq!(
            described(&dict),
            [
                (EntryPointKind::Script, "pkg.cli".into(), "main".into()),
                (EntryPointKind::GuiScript, "pkg.gui".into(), "launch".into()),
                (EntryPointKind::Plugin, "pkg.plugin".into(), String::new()),
            ]
        );

        let ini = super::setup_py_entry_points(
            "setup(entry_points=\"\"\"\n[console_scripts]\nlegacy = pkg.cli:main\n\n[pytest11]\nlegacy = pkg.plugin\n\"\"\")\n",
        );
        assert_eq!(
            described(&ini),
            [
                (EntryPointKind::Script, "pkg.cli".into(), "main".into()),
                (EntryPointKind::Plugin, "pkg.plugin".into(), String::new()),
            ]
        );

        let cfg = super::setup_cfg_entry_points(
            "[metadata]\nname = legacy\n\n[options.entry_points]\nconsole_scripts =\n    legacy = pkg.cli:main\n    other = pkg.other:run\ngui_scripts = legacy-gui = pkg.gui:launch\n\n[options]\nzip_safe = False\n",
        );
        assert_eq!(
            described(&cfg),
            [
                (EntryPointKind::Script, "pkg.cli".into(), "main".into()),
                (EntryPointKind::Script, "pkg.other".into(), "run".into()),
                (EntryPointKind::GuiScript, "pkg.gui".into(), "launch".into()),
            ],
            "a group may open a block or declare one target on its own line"
        );
    }

    #[test]
    fn reads_install_requires_from_setup_py_and_setup_cfg() {
        let py = super::setup_py_dependencies(
            "from setuptools import setup\nsetup(name='x', install_requires=['six>=1', 'Pillow'], extras_require={'dev': ['pytest']})\n",
        );
        let names: Vec<_> = py
            .iter()
            .map(|d| (d.name.as_str(), d.group.clone()))
            .collect();
        assert_eq!(
            names,
            [
                ("six", DependencyGroup::Main),
                ("pillow", DependencyGroup::Main),
                ("pytest", DependencyGroup::Optional("dev".into())),
            ]
        );
        let cfg = super::setup_cfg_dependencies(
            "[metadata]\nname = x\n[options]\ninstall_requires =\n    six\n    requests>=2\nzip_safe = false\n",
        );
        let names: Vec<_> = cfg.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["six", "requests"]);
    }

    #[test]
    fn reads_health_thresholds_with_defaults_for_the_rest() {
        let settings = parse("[tool.pyscythe.health]\nmax-cyclomatic = 4\n").unwrap();
        assert_eq!(settings.config.health.max_cyclomatic, 4);
        assert_eq!(settings.config.health.max_cognitive, 15);
    }

    #[test]
    fn empty_or_unknown_boundaries_are_errors() {
        assert!(parse("[tool.pyscythe.boundaries]\n").is_err());
        assert!(parse("[tool.pyscythe.boundaries]\npreset = \"onion\"\nroot = \"app\"\n").is_err());
        assert!(parse("[tool.pyscythe.boundaries]\npreset = \"hexagonal\"\n").is_err());
    }

    #[test]
    fn unknown_tool_keys_are_rejected() {
        let error = parse("[tool.pyscythe]\nexcludes = []\n").unwrap_err();
        assert!(error.to_string().contains("excludes"), "{error}");
    }

    #[test]
    fn a_bad_glob_is_an_error() {
        assert!(parse("[tool.pyscythe]\nexclude = [\"[oops\"]\n").is_err());
    }
}
