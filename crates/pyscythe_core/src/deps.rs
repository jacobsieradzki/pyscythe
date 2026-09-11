//! Compares what `pyproject.toml` declares with what the code imports.

use std::collections::{BTreeMap, BTreeSet};

use camino::Utf8Path;

use crate::config::Config;
use crate::finding::{Confidence, Detail, Finding, Rule};
use crate::index::{CodebaseIndex, ImportOrigin};
use crate::manifest::{DependencyGroup, DistributionName, Manifest};
use crate::report::{Report, ReportKind, Summary};

/// Distributions that are run as commands or loaded by other tools rather
/// than imported, so never "unused".
const TOOL_DISTRIBUTIONS: &[&str] = &[
    "black",
    "build",
    "coverage",
    "flake8",
    "gunicorn",
    "hypercorn",
    "ipykernel",
    "isort",
    "jupyter",
    "jupyterlab",
    "maturin",
    "mypy",
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

/// Runs the dependency analysis: declared-but-unused, imported-but-undeclared,
/// and unresolved imports.
#[must_use]
pub fn analyze(
    index: &dyn CodebaseIndex,
    manifest: &Manifest,
    config: &Config,
    root: &Utf8Path,
) -> Report {
    let files = index.files();
    let declared: BTreeSet<&DistributionName> =
        manifest.dependencies.iter().map(|d| &d.name).collect();
    let survey = survey_imports(index, &declared);

    let mut findings: Vec<Finding> = manifest
        .dependencies
        .iter()
        .filter(|dependency| !survey.used.contains_key(&dependency.name))
        .filter(|dependency| !is_tool(&dependency.name, config))
        .map(|dependency| unused_finding(dependency, &root.join("pyproject.toml")))
        .collect();
    for (distribution, (modules, mut finding)) in survey.undeclared {
        let names: Vec<String> = modules.into_iter().collect();
        finding.message = format!(
            "`{}` is imported but `{}` is not a declared dependency",
            names.join("`, `"),
            distribution.as_str()
        );
        finding.detail = Detail::Dependency {
            distribution: distribution.as_str().to_owned(),
            modules: names,
        };
        findings.push(finding);
    }
    findings.extend(survey.unresolved.into_values());
    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.position.cmp(&b.position)));

    Report {
        schema_version: Report::SCHEMA_VERSION,
        kind: ReportKind::Deps,
        summary: Summary {
            files_scanned: files.len(),
            symbols_checked: manifest.dependencies.len(),
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

fn survey_imports(
    index: &dyn CodebaseIndex,
    declared: &BTreeSet<&DistributionName>,
) -> ImportSurvey {
    let mut survey = ImportSurvey::default();
    for file in index.files() {
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

fn unused_finding(dependency: &crate::manifest::Dependency, pyproject: &Utf8Path) -> Finding {
    let group = match &dependency.group {
        DependencyGroup::Main => String::new(),
        DependencyGroup::Optional(name) => format!(" (optional-dependencies.{name})"),
        DependencyGroup::Group(name) => format!(" (dependency-groups.{name})"),
    };
    Finding {
        rule: Rule::UnusedDependency,
        path: pyproject.to_path_buf(),
        module: None,
        position: None,
        confidence: Confidence::Medium,
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

/// Tools, `types-*` stub packages (never imported), and configured exemptions.
fn is_tool(name: &DistributionName, config: &Config) -> bool {
    TOOL_DISTRIBUTIONS.contains(&name.as_str())
        || name.as_str().starts_with("types-")
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

    fn rules(report: &crate::report::Report) -> Vec<(Rule, String)> {
        report
            .findings
            .iter()
            .map(|f| (f.rule, f.message.clone()))
            .collect()
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
            &manifest(&["pydantic", "requests", "ruff"]),
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
            &manifest(&["Pillow", "typing-extensions"]),
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
    fn ignored_and_tool_dependencies_are_not_reported() {
        let mut index = FakeIndex::new();
        index.add_file("/proj/pkg/a.py", "pkg.a");
        let config = Config {
            ignored_dependencies: vec![DistributionName::normalize("my-plugin")],
            ..Config::default()
        };

        let report = analyze(
            &index,
            &manifest(&["ruff", "my_plugin"]),
            &config,
            Utf8Path::new("/proj"),
        );

        assert!(report.is_clean(), "{:?}", report.findings);
    }
}
