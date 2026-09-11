//! Turns `pyproject.toml` into a [`Manifest`] and a [`Config`].

use std::collections::BTreeMap;

use camino::Utf8Path;
use pyscythe_core::config::{
    BoundaryConfig, Config, DenyRule, HealthThresholds, ModulePrefix, NamePatterns, NotebookPolicy,
    PathPatterns, PatternError, TypeOnlyImports,
};
use pyscythe_core::manifest::{
    Dependency, DependencyGroup, DistributionName, EntryPoint, EntryPointKind, Manifest,
};
use pyscythe_core::source::ModulePath;
use pyscythe_core::symbol::SymbolName;
use serde::Deserialize;

/// What `pyproject.toml` told us.
#[derive(Debug, Clone)]
pub struct ProjectSettings {
    /// Entry points, including extras from `[tool.pyscythe]`.
    pub manifest: Manifest,
    /// User configuration from `[tool.pyscythe]`.
    pub config: Config,
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
/// A project without a `pyproject.toml` yields empty settings.
///
/// # Errors
///
/// Returns [`ManifestError`] when the file exists but cannot be read or parsed.
pub fn load(root: &Utf8Path) -> Result<ProjectSettings, ManifestError> {
    let path = root.join("pyproject.toml");
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text).map_err(|source| ManifestError::Parse { path, source }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(ProjectSettings {
            manifest: Manifest::empty(),
            config: Config::default(),
        }),
        Err(source) => Err(ManifestError::Io { path, source }),
    }
}

/// Parses `pyproject.toml` text.
///
/// # Errors
///
/// Returns [`ParseError`] when the text is malformed or a glob is invalid.
pub fn parse(text: &str) -> Result<ProjectSettings, ParseError> {
    let document: PyProject = toml::from_str(text)?;
    let project = document.project.unwrap_or_default();
    let tool = document
        .tool
        .and_then(|tool| tool.pyscythe)
        .unwrap_or_default();

    let mut entry_points = Vec::new();
    entry_points.extend(targets(&project.scripts, EntryPointKind::Script));
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
        ignored_dependencies: tool
            .deps
            .unwrap_or_default()
            .ignore
            .iter()
            .map(|name| DistributionName::normalize(name))
            .collect(),
        health: {
            let defaults = HealthThresholds::default();
            let table = tool.health.unwrap_or_default();
            HealthThresholds {
                max_cyclomatic: table.cyclomatic.unwrap_or(defaults.max_cyclomatic),
                max_cognitive: table.cognitive.unwrap_or(defaults.max_cognitive),
                max_lines: table.lines.unwrap_or(defaults.max_lines),
                max_parameters: table.parameters.unwrap_or(defaults.max_parameters),
            }
        },
    };

    Ok(ProjectSettings {
        manifest: Manifest {
            entry_points,
            dependencies,
        },
        config,
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

#[derive(Debug, Deserialize)]
struct Tool {
    pyscythe: Option<PyscytheTable>,
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
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Project {
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
