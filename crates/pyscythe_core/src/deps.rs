//! Compares what `pyproject.toml` declares with what the code imports.

use std::collections::{BTreeMap, BTreeSet};

use camino::{Utf8Path, Utf8PathBuf};

use crate::config::Config;
use crate::finding::{Confidence, Detail, Finding, Rule};
use crate::index::{CodebaseIndex, ImportOrigin};
use crate::manifest::{DependencyGroup, DistributionName, Manifest};
use crate::report::{Report, ReportKind, Summary};

/// Distributions that are run as commands or loaded by other tools rather
/// than imported, so never "unused".
const TOOL_DISTRIBUTIONS: &[&str] = &[
    "bandit",
    "black",
    "build",
    "bump2version",
    "bumpversion",
    "cibuildwheel",
    "codespell",
    "commitizen",
    "coverage",
    "flake8",
    "gunicorn",
    "hatch",
    "hypercorn",
    "ipdb",
    "ipykernel",
    "ipython",
    "isort",
    "jupyter",
    "jupyterlab",
    "maturin",
    "mkdocs",
    "mkdocs-material",
    "mypy",
    "pdm",
    "poetry",
    "poethepoet",
    "pyinstaller",
    "pylint",
    "pyupgrade",
    "sphinx",
    "notebook",
    "nox",
    "pip",
    "pip-tools",
    "pre-commit",
    "pyright",
    "pytest",
    "pytest-asyncio",
    "pytest-cov",
    "pytest-mock",
    "pytest-xdist",
    "ruff",
    "setuptools",
    "tox",
    "twine",
    "ty",
    "uv",
    "uvicorn",
    "wheel",
];

/// Distributions a framework loads by configuration rather than an import:
/// database drivers named in a URL, validators `FastAPI` and Pydantic pull in
/// when a field asks for them, servers' event loops and parsers.
const LOADED_BY_CONFIGURATION: &[&str] = &[
    "aiosqlite",
    "asyncpg",
    "cx-oracle",
    "email-validator",
    "httptools",
    "mysqlclient",
    "oracledb",
    "pg8000",
    "psycopg",
    "psycopg-binary",
    "psycopg-c",
    "psycopg2",
    "psycopg2-binary",
    "pymysql",
    "pyodbc",
    "python-multipart",
    "uvloop",
    "watchfiles",
];

/// Distributions whose import name does not follow from their name.
const IMPORT_ALIASES: &[(&str, &str)] = &[
    ("attrs", "attr"),
    ("beautifulsoup4", "bs4"),
    ("google-cloud-storage", "google"),
    ("opencv-python", "cv2"),
    ("pillow", "PIL"),
    ("psycopg2-binary", "psycopg2"),
    ("pyjwt", "jwt"),
    ("python-dateutil", "dateutil"),
    ("python-dotenv", "dotenv"),
    ("pyyaml", "yaml"),
    ("scikit-learn", "sklearn"),
    ("sqlalchemy", "sqlalchemy"),
];

/// What the project's imports say about its dependencies.
#[derive(Default)]
struct ImportSurvey {
    /// Distributions some import resolved into, with the modules that did it.
    used: BTreeMap<DistributionName, BTreeSet<String>>,
    /// Distributions imported but not declared: modules and the first import seen.
    undeclared: BTreeMap<DistributionName, (BTreeSet<String>, Finding)>,
    /// Modules nothing provides, keyed by name, with the first import seen.
    unresolved: BTreeMap<String, Finding>,
}

/// A manifest and the directory it governs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestScope {
    /// The `pyproject.toml`, `setup.py`, or `setup.cfg` that declares the dependencies.
    pub manifest_path: Utf8PathBuf,
    /// What it declares.
    pub manifest: Manifest,
    /// The project's own distribution name, when the manifest says.
    pub name: Option<DistributionName>,
}

impl ManifestScope {
    /// The directory the manifest governs.
    #[must_use]
    pub fn directory(&self) -> &Utf8Path {
        self.manifest_path
            .parent()
            .unwrap_or_else(|| Utf8Path::new(""))
    }
}

/// Runs the dependency analysis: declared-but-unused, imported-but-undeclared,
/// and unresolved imports.
///
/// Each file is judged against the deepest manifest
/// whose directory contains it, so a `backend/pyproject.toml` governs
/// `backend/`, and files outside every manifest fall to the first scope.
#[must_use]
pub fn analyze(
    index: &dyn CodebaseIndex,
    scopes: &[ManifestScope],
    config: &Config,
    root: &Utf8Path,
) -> Report {
    let files = index.files();
    let fallback = ManifestScope {
        manifest_path: root.join("pyproject.toml"),
        manifest: Manifest::empty(),
        name: None,
    };
    let scope_of = |file: &crate::source::SourceFile| -> &ManifestScope {
        scopes
            .iter()
            .filter(|scope| file.path.starts_with(scope.directory()))
            .max_by_key(|scope| scope.directory().as_str().len())
            .or_else(|| scopes.first())
            .unwrap_or(&fallback)
    };

    let mut findings: Vec<Finding> = Vec::new();
    let mut surveyed: Vec<(&ManifestScope, ImportSurvey, BTreeSet<&DistributionName>)> = Vec::new();
    for scope in scopes.iter().chain(std::iter::once(&fallback)) {
        let declared: BTreeSet<&DistributionName> = scope
            .manifest
            .dependencies
            .iter()
            .map(|d| &d.name)
            .collect();
        let members: Vec<&crate::source::SourceFile> = files
            .iter()
            .filter(|file| std::ptr::eq(scope_of(file), scope))
            .collect();
        if members.is_empty() && scope.manifest.dependencies.is_empty() {
            continue;
        }
        let survey = survey_imports(index, &members, &declared);
        surveyed.push((scope, survey, declared));
    }

    for (scope, survey, declared) in surveyed {
        // A name declared in several groups is one dependency; `celery[redis]`
        // in celery's own extras names the project itself.
        let mut reported: BTreeSet<&DistributionName> = BTreeSet::new();
        findings.extend(
            scope
                .manifest
                .dependencies
                .iter()
                .filter(|dependency| !survey.used.contains_key(&dependency.name))
                .filter(|dependency| !is_tool(&dependency.name, config))
                .filter(|dependency| scope.name.as_ref() != Some(&dependency.name))
                .filter(|dependency| reported.insert(&dependency.name))
                .map(|dependency| unused_finding(dependency, &scope.manifest_path)),
        );
        for (distribution, (modules, mut finding)) in survey.undeclared {
            // An example or script importing the project's own package is not
            // a dependency question, however the editable install resolves.
            if is_own_package(scope, distribution.as_str()) {
                continue;
            }
            let names: Vec<String> = modules.into_iter().collect();
            let imported = format!("`{}` is imported but", names.join("`, `"));
            finding.message = match declared_carrier(index, &declared, &distribution) {
                Some(carrier) => {
                    // Present today only because something declared pulls it in.
                    finding.confidence = Confidence::Low;
                    format!(
                        "{imported} `{}` is not a declared dependency; it arrives through `{}`",
                        distribution.as_str(),
                        carrier.as_str()
                    )
                }
                None => format!(
                    "{imported} `{}` is not a declared dependency",
                    distribution.as_str()
                ),
            };
            finding.detail = Detail::Dependency {
                distribution: distribution.as_str().to_owned(),
                modules: names,
            };
            findings.push(finding);
        }
        findings.extend(
            survey
                .unresolved
                .into_iter()
                .filter(|(module, _)| !is_own_package(scope, module))
                .map(|(_, finding)| finding),
        );
    }
    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.position.cmp(&b.position)));

    Report {
        schema_version: Report::SCHEMA_VERSION,
        kind: ReportKind::Deps,
        summary: Summary {
            files_scanned: files.len(),
            symbols_checked: scopes.iter().map(|s| s.manifest.dependencies.len()).sum(),
            symbols_kept: 0,
            symbols_ignored: 0,
            suppressed: 0,
            baselined: 0,
            findings: findings.len(),
            changed_files: None,
            health: None,
            duplication: None,
        },
        findings,
        kept: Vec::new(),
    }
}

/// Whether `module` names the project's own distribution, which an example or
/// a script inside the project may import like any other package.
fn is_own_package(scope: &ManifestScope, module: &str) -> bool {
    scope.name.as_ref() == Some(&DistributionName::normalize(module))
}

/// The declared dependency whose own requirements, transitively, bring in
/// `wanted`: `fastapi` for an import of `starlette`.
fn declared_carrier(
    index: &dyn CodebaseIndex,
    declared: &BTreeSet<&DistributionName>,
    wanted: &DistributionName,
) -> Option<DistributionName> {
    const MAX_VISITED: usize = 500;
    declared.iter().find_map(|&root| {
        let mut seen: BTreeSet<DistributionName> = BTreeSet::new();
        let mut pending = vec![root.clone()];
        while let Some(current) = pending.pop() {
            if seen.len() >= MAX_VISITED || !seen.insert(current.clone()) {
                continue;
            }
            for requirement in index.distribution_requirements(&current) {
                if &requirement == wanted {
                    return Some(root.clone());
                }
                pending.push(requirement);
            }
        }
        None
    })
}

fn survey_imports(
    index: &dyn CodebaseIndex,
    files: &[&crate::source::SourceFile],
    declared: &BTreeSet<&DistributionName>,
) -> ImportSurvey {
    let mut survey = ImportSurvey::default();
    for &file in files {
        for import in index.external_imports(file.id) {
            let position = index.position(file.id, import.span.start());
            let placeholder = || Finding {
                rule: Rule::MissingDependency,
                path: file.path.clone(),
                module: file.module.clone(),
                position,
                confidence: Confidence::Medium,
                message: String::new(),
                detail: Detail::File,
            };
            match &import.origin {
                ImportOrigin::SitePackages { distributions } if !distributions.is_empty() => {
                    for distribution in distributions {
                        survey
                            .used
                            .entry(distribution.clone())
                            .or_default()
                            .insert(import.top_level.clone());
                        if !declared.contains(distribution) {
                            survey
                                .undeclared
                                .entry(distribution.clone())
                                .or_insert_with(|| (BTreeSet::new(), placeholder()))
                                .0
                                .insert(import.top_level.clone());
                        }
                    }
                }
                ImportOrigin::SitePackages { .. } | ImportOrigin::Unresolved => {
                    // No metadata: match the module name against declared names.
                    let module = import.top_level.clone();
                    if let Some(distribution) = declared.iter().find(|d| provides(d, &module)) {
                        survey
                            .used
                            .entry((*distribution).clone())
                            .or_default()
                            .insert(module);
                    } else if import.origin == ImportOrigin::Unresolved {
                        survey
                            .unresolved
                            .entry(module.clone())
                            .or_insert_with(|| Finding {
                                rule: Rule::UnresolvedImport,
                                confidence: Confidence::Low,
                                message: format!(
                                    "import `{module}` resolves to nothing on the search path"
                                ),
                                detail: Detail::Dependency {
                                    distribution: String::new(),
                                    modules: vec![module.clone()],
                                },
                                ..placeholder()
                            });
                    }
                }
            }
        }
    }
    survey
}

fn unused_finding(dependency: &crate::manifest::Dependency, manifest_path: &Utf8Path) -> Finding {
    let group = match &dependency.group {
        DependencyGroup::Main => String::new(),
        DependencyGroup::Optional(name) => format!(" (optional-dependencies.{name})"),
        DependencyGroup::Group(name) => format!(" (dependency-groups.{name})"),
    };
    // Development groups hold runners and plugins that are rarely imported,
    // and extras are imported lazily, by name, when a user installs them.
    let confidence = match dependency.group {
        DependencyGroup::Main => Confidence::Medium,
        DependencyGroup::Optional(_) | DependencyGroup::Group(_) => Confidence::Low,
    };
    Finding {
        rule: Rule::UnusedDependency,
        path: manifest_path.to_path_buf(),
        module: None,
        position: None,
        confidence,
        message: format!(
            "dependency `{}`{group} is never imported",
            dependency.name.as_str()
        ),
        detail: Detail::Dependency {
            distribution: dependency.name.as_str().to_owned(),
            modules: Vec::new(),
        },
    }
}

/// Tools, pytest plugins, stub packages (never imported), drivers loaded by
/// configuration, and configured exemptions.
fn is_tool(name: &DistributionName, config: &Config) -> bool {
    let name_str = name.as_str();
    TOOL_DISTRIBUTIONS.contains(&name_str)
        || LOADED_BY_CONFIGURATION.contains(&name_str)
        || name_str.starts_with("types-")
        || name_str.ends_with("-stubs")
        || name_str.starts_with("pytest-")
        || config.ignored_dependencies.contains(name)
}

/// Whether `distribution` plausibly provides top-level `module`, by convention or alias.
fn provides(distribution: &DistributionName, module: &str) -> bool {
    if distribution
        .conventional_module()
        .eq_ignore_ascii_case(module)
    {
        return true;
    }
    IMPORT_ALIASES
        .iter()
        .any(|(dist, alias)| *dist == distribution.as_str() && *alias == module)
}

#[cfg(test)]
mod tests {
    use camino::Utf8Path;

    use super::analyze;
    use crate::config::Config;
    use crate::finding::Rule;
    use crate::index::ImportOrigin;
    use crate::manifest::{Dependency, DependencyGroup, DistributionName, Manifest};
    use crate::testing::FakeIndex;

    fn manifest(names: &[&str]) -> Manifest {
        Manifest {
            entry_points: Vec::new(),
            dependencies: names
                .iter()
                .map(|n| Dependency {
                    name: DistributionName::normalize(n),
                    group: DependencyGroup::Main,
                })
                .collect(),
        }
    }

    fn scopes(names: &[&str]) -> Vec<super::ManifestScope> {
        vec![super::ManifestScope {
            manifest_path: camino::Utf8PathBuf::from("/proj/pyproject.toml"),
            manifest: manifest(names),
            name: None,
        }]
    }

    fn rules(report: &crate::report::Report) -> Vec<(Rule, String)> {
        report
            .findings
            .iter()
            .map(|f| (f.rule, f.message.clone()))
            .collect()
    }

    #[test]
    fn a_project_importing_its_own_package_is_not_missing_a_dependency() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/examples/demo.py", "examples.demo");
        index.add_external_import(
            file,
            "openai",
            ImportOrigin::SitePackages {
                distributions: vec![DistributionName::normalize("openai")],
            },
        );
        let unresolved = index.add_file("/proj/examples/other.py", "examples.other");
        index.add_external_import(unresolved, "openai", ImportOrigin::Unresolved);
        let mut scopes = scopes(&["httpx"]);
        scopes[0].name = Some(DistributionName::normalize("openai"));

        let report = analyze(&index, &scopes, &Config::default(), Utf8Path::new("/proj"));

        assert!(
            !report.findings.iter().any(|finding| matches!(
                finding.rule,
                Rule::MissingDependency | Rule::UnresolvedImport
            )),
            "an example importing the project itself is neither missing nor unresolved: {:?}",
            rules(&report)
        );
    }

    #[test]
    fn metadata_backed_imports_mark_dependencies_used_and_flag_undeclared_ones() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/a.py", "pkg.a");
        index.add_external_import(
            file,
            "pydantic",
            ImportOrigin::SitePackages {
                distributions: vec![DistributionName::normalize("pydantic")],
            },
        );
        index.add_external_import(
            file,
            "httpx",
            ImportOrigin::SitePackages {
                distributions: vec![DistributionName::normalize("httpx")],
            },
        );

        let report = analyze(
            &index,
            &scopes(&["pydantic", "requests", "ruff"]),
            &Config::default(),
            Utf8Path::new("/proj"),
        );

        assert_eq!(
            rules(&report),
            [
                (
                    Rule::MissingDependency,
                    "`httpx` is imported but `httpx` is not a declared dependency".to_owned()
                ),
                (
                    Rule::UnusedDependency,
                    "dependency `requests` is never imported".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn without_metadata_names_and_aliases_are_matched_and_unknowns_reported() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/a.py", "pkg.a");
        index.add_external_import(file, "PIL", ImportOrigin::Unresolved);
        index.add_external_import(file, "typing_extensions", ImportOrigin::Unresolved);
        index.add_external_import(file, "mystery", ImportOrigin::Unresolved);

        let report = analyze(
            &index,
            &scopes(&["Pillow", "typing-extensions"]),
            &Config::default(),
            Utf8Path::new("/proj"),
        );

        assert_eq!(
            rules(&report),
            [(
                Rule::UnresolvedImport,
                "import `mystery` resolves to nothing on the search path".to_owned()
            )]
        );
    }

    #[test]
    fn a_project_naming_itself_in_extras_and_repeated_declarations_are_not_unused() {
        let index = FakeIndex::new();
        let mut manifest = manifest(&["celery", "six"]);
        manifest.dependencies.push(Dependency {
            name: DistributionName::normalize("six"),
            group: DependencyGroup::Optional("extra".into()),
        });
        let scopes = vec![super::ManifestScope {
            manifest_path: camino::Utf8PathBuf::from("/proj/pyproject.toml"),
            manifest,
            name: Some(DistributionName::normalize("celery")),
        }];

        let report = analyze(&index, &scopes, &Config::default(), Utf8Path::new("/proj"));

        assert_eq!(
            rules(&report),
            [(
                Rule::UnusedDependency,
                "dependency `six` is never imported".to_owned()
            )]
        );
    }

    #[test]
    fn imports_reachable_through_a_declared_dependency_are_low_confidence() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/app/main.py", "app.main");
        index.add_external_import(
            file,
            "starlette",
            ImportOrigin::SitePackages {
                distributions: vec![DistributionName::normalize("starlette")],
            },
        );
        index.add_requirement("fastapi", "starlette");

        let report = analyze(
            &index,
            &scopes(&["fastapi"]),
            &Config::default(),
            Utf8Path::new("/proj"),
        );

        let finding = report
            .findings
            .iter()
            .find(|f| f.rule == Rule::MissingDependency)
            .expect("starlette is undeclared");
        assert_eq!(finding.confidence, crate::finding::Confidence::Low);
        assert!(
            finding.message.ends_with("it arrives through `fastapi`"),
            "{}",
            finding.message
        );
    }

    #[test]
    fn each_file_is_judged_against_the_nearest_manifest() {
        let mut index = FakeIndex::new();
        let backend = index.add_file("/proj/backend/app/main.py", "app.main");
        let script = index.add_file("/proj/scripts/tool.py", "scripts.tool");
        let six = || ImportOrigin::SitePackages {
            distributions: vec![DistributionName::normalize("six")],
        };
        index.add_external_import(backend, "six", six());
        index.add_external_import(script, "six", six());
        let scopes = vec![
            super::ManifestScope {
                manifest_path: camino::Utf8PathBuf::from("/proj/pyproject.toml"),
                manifest: manifest(&[]),
                name: None,
            },
            super::ManifestScope {
                manifest_path: camino::Utf8PathBuf::from("/proj/backend/pyproject.toml"),
                manifest: manifest(&["six", "unused-lib"]),
                name: None,
            },
        ];

        let report = analyze(&index, &scopes, &Config::default(), Utf8Path::new("/proj"));

        let summary: Vec<(Rule, String)> = report
            .findings
            .iter()
            .map(|f| (f.rule, f.path.to_string()))
            .collect();
        assert_eq!(
            summary,
            [
                (
                    Rule::UnusedDependency,
                    "/proj/backend/pyproject.toml".to_owned()
                ),
                (Rule::MissingDependency, "/proj/scripts/tool.py".to_owned()),
            ],
            "{:?}",
            report.findings
        );
    }

    #[test]
    fn ignored_and_tool_dependencies_are_not_reported() {
        let mut index = FakeIndex::new();
        index.add_file("/proj/pkg/a.py", "pkg.a");
        let config = Config {
            ignored_dependencies: vec![DistributionName::normalize("my-plugin")],
            ..Config::default()
        };

        let report = analyze(
            &index,
            &scopes(&["ruff", "my_plugin"]),
            &config,
            Utf8Path::new("/proj"),
        );

        assert!(report.is_clean(), "{:?}", report.findings);
    }
}
