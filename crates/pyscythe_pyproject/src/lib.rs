//! Turns `pyproject.toml` into a [`Manifest`].

use std::collections::BTreeMap;

use camino::Utf8Path;
use pyscythe_core::manifest::{EntryPoint, EntryPointKind, Manifest};
use pyscythe_core::source::ModulePath;
use pyscythe_core::symbol::SymbolName;
use serde::Deserialize;

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
        source: toml::de::Error,
    },
}

/// Loads the manifest for the project rooted at `root`.
///
/// A project without a `pyproject.toml` yields an empty manifest.
///
/// # Errors
///
/// Returns [`ManifestError`] when the file exists but cannot be read or parsed.
pub fn load(root: &Utf8Path) -> Result<Manifest, ManifestError> {
    let path = root.join("pyproject.toml");
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text).map_err(|source| ManifestError::Parse { path, source }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::empty()),
        Err(source) => Err(ManifestError::Io { path, source }),
    }
}

/// Parses `pyproject.toml` text.
///
/// # Errors
///
/// Returns the TOML error when the text is malformed.
pub fn parse(text: &str) -> Result<Manifest, toml::de::Error> {
    let document: PyProject = toml::from_str(text)?;
    let project = document.project.unwrap_or_default();

    let mut entry_points = Vec::new();
    entry_points.extend(targets(&project.scripts, EntryPointKind::Script));
    entry_points.extend(targets(&project.gui_scripts, EntryPointKind::GuiScript));
    for group in project.entry_points.values() {
        entry_points.extend(targets(group, EntryPointKind::Plugin));
    }

    Ok(Manifest { entry_points })
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
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Project {
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
    use pyscythe_core::manifest::EntryPointKind;
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
        let manifest = parse("[project]\nname = \"demo\"\n").unwrap();
        assert!(manifest.entry_points.is_empty());
    }

    #[test]
    fn a_file_without_a_project_table_is_empty() {
        let manifest = parse("[tool.ruff]\nline-length = 100\n").unwrap();
        assert!(manifest.entry_points.is_empty());
    }

    #[test]
    fn malformed_toml_is_an_error() {
        assert!(parse("[project\n").is_err());
    }
}
