//! The corpus manifest: which projects to analyse, at which commit, in which environment.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::str::FromStr;

use anyhow::{Context as _, Result, bail};
use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use serde::Deserialize;

/// A project's key in the manifest, also its directory name under the cache and the snapshots.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct ProjectName(String);

impl ProjectName {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ProjectName {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self> {
        let valid = !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !valid {
            bail!("project name `{value}` must be lowercase letters, digits, and hyphens");
        }
        Ok(Self(value))
    }
}

impl FromStr for ProjectName {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::try_from(value.to_owned())
    }
}

impl fmt::Display for ProjectName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&self.0)
    }
}

/// A full 40-character git commit hash.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct CommitSha(String);

impl CommitSha {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CommitSha {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self> {
        let valid = value.len() == 40
            && value
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
        if !valid {
            bail!("commit `{value}` must be a full lowercase 40-character hash");
        }
        Ok(Self(value))
    }
}

/// A `major.minor` Python version, the argument to uv's `--python`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct PythonVersion(String);

impl PythonVersion {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PythonVersion {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self> {
        let valid = match value.split_once('.') {
            Some((major, minor)) => {
                !major.is_empty()
                    && !minor.is_empty()
                    && major.chars().all(|c| c.is_ascii_digit())
                    && minor.chars().all(|c| c.is_ascii_digit())
            }
            None => false,
        };
        if !valid {
            bail!("python version `{value}` must be `major.minor`, such as `3.12`");
        }
        Ok(Self(value))
    }
}

/// Where a project's git history is fetched from.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct Repository(String);

impl Repository {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Repository {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self> {
        if !value.starts_with("https://") {
            bail!("repository `{value}` must be an https URL");
        }
        Ok(Self(value))
    }
}

/// One line of the environment's requirements: a specifier, or `-r` a file inside the clone.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub(crate) enum Requirement {
    /// A PEP 508 requirement such as `pytest` or `pandas==3.0.0`.
    Specifier(String),
    /// A requirements file, relative to the project's root inside the clone.
    File(Utf8PathBuf),
}

impl TryFrom<String> for Requirement {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self> {
        if let Some(file) = value.strip_prefix("-r ") {
            return Ok(Self::File(Utf8PathBuf::from(file.trim())));
        }
        if value.starts_with('-') {
            bail!("requirement `{value}`: only `-r <file>` options are supported");
        }
        Ok(Self::Specifier(value))
    }
}

/// One pinned project.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Project {
    pub(crate) repository: Repository,
    pub(crate) commit: CommitSha,
    pub(crate) python: PythonVersion,
    /// Packages the environment needs beyond the project itself.
    #[serde(default)]
    pub(crate) requirements: Vec<Requirement>,
    /// Directories inside the clone installed editable, without their dependencies.
    #[serde(default)]
    pub(crate) editable: Vec<Utf8PathBuf>,
    /// The directory inside the clone that pyscythe analyses; the environment lives there.
    #[serde(default = "default_root")]
    pub(crate) root: Utf8PathBuf,
}

fn default_root() -> Utf8PathBuf {
    Utf8PathBuf::from(".")
}

/// Every pinned project, keyed by name.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub(crate) projects: BTreeMap<ProjectName, Project>,
}

impl Manifest {
    pub(crate) fn load(path: &Utf8Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
        let manifest: Self = toml::from_str(&text).with_context(|| format!("parsing {path}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<()> {
        for (name, project) in &self.projects {
            let inside = project
                .editable
                .iter()
                .chain(std::iter::once(&project.root));
            for path in inside {
                if !stays_inside(path) {
                    bail!("project `{name}`: path `{path}` must stay inside the clone");
                }
            }
            for requirement in &project.requirements {
                if let Requirement::File(path) = requirement
                    && !stays_inside(path)
                {
                    bail!(
                        "project `{name}`: requirements file `{path}` must stay inside the clone"
                    );
                }
            }
        }
        Ok(())
    }
}

fn stays_inside(path: &Utf8Path) -> bool {
    path.is_relative()
        && path
            .components()
            .all(|component| matches!(component, Utf8Component::Normal(_) | Utf8Component::CurDir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_project_with_defaults() {
        let manifest: Manifest = toml::from_str(
            r#"
[projects.django]
repository = "https://github.com/django/django"
commit = "2b30f6255b5ef84afbd827993643d52ef2c0963a"
python = "3.12"
requirements = ["pytest", "-r requirements/base.txt"]
editable = ["."]
"#,
        )
        .unwrap();
        let project = &manifest.projects[&ProjectName::from_str("django").unwrap()];
        assert_eq!(project.root, Utf8PathBuf::from("."));
        assert_eq!(
            project.requirements,
            vec![
                Requirement::Specifier("pytest".into()),
                Requirement::File("requirements/base.txt".into())
            ]
        );
        manifest.validate().unwrap();
    }

    #[test]
    fn rejects_short_commits_and_escaping_paths() {
        assert!(CommitSha::try_from("2b30f62".to_owned()).is_err());
        assert!(PythonVersion::try_from("3".to_owned()).is_err());
        assert!(ProjectName::try_from("Django".to_owned()).is_err());
        let manifest: Manifest = toml::from_str(
            r#"
[projects.escaping]
repository = "https://example.invalid/repo"
commit = "2b30f6255b5ef84afbd827993643d52ef2c0963a"
python = "3.12"
root = "../elsewhere"
"#,
        )
        .unwrap();
        assert!(manifest.validate().is_err());
    }
}
