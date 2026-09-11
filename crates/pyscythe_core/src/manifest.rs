//! What the project declares about itself, independent of any source file.

use crate::source::ModulePath;
use crate::symbol::SymbolName;

/// Where an entry point was declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryPointKind {
    /// `[project.scripts]`.
    Script,
    /// `[project.gui-scripts]`.
    GuiScript,
    /// `[project.entry-points.<group>]`.
    Plugin,
}

/// A `module:attribute` target that an installer or plugin host will import.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntryPoint {
    /// Where it was declared.
    pub kind: EntryPointKind,
    /// The module to import.
    pub module: ModulePath,
    /// The attribute to load from it, when one is named.
    pub attribute: Option<SymbolName>,
}

/// A PEP 503 normalised distribution name: lowercase, runs of `-_.` collapsed to `-`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
#[serde(transparent)]
pub struct DistributionName(String);

impl DistributionName {
    /// Normalises `raw`, which may be a requirement's project name in any spelling.
    #[must_use]
    pub fn normalize(raw: &str) -> Self {
        let mut out = String::with_capacity(raw.len());
        let mut pending_separator = false;
        for c in raw.trim().chars() {
            if matches!(c, '-' | '_' | '.') {
                pending_separator = !out.is_empty();
            } else {
                if pending_separator {
                    out.push('-');
                    pending_separator = false;
                }
                out.extend(c.to_lowercase());
            }
        }
        Self(out)
    }

    /// The normalised name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The import name this distribution would have if it followed the usual
    /// convention: the name with `-` as `_`.
    #[must_use]
    pub fn conventional_module(&self) -> String {
        self.0.replace('-', "_")
    }
}

/// Where a dependency was declared.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyGroup {
    /// `[project.dependencies]`.
    Main,
    /// `[project.optional-dependencies.<name>]`.
    Optional(String),
    /// PEP 735 `[dependency-groups.<name>]`.
    Group(String),
}

/// One declared requirement.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Dependency {
    /// The distribution, normalised.
    pub name: DistributionName,
    /// Where it was declared.
    pub group: DependencyGroup,
}

/// Facts from `pyproject.toml` and friends that analyses need.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// Every declared entry point.
    pub entry_points: Vec<EntryPoint>,
    /// Every declared requirement.
    pub dependencies: Vec<Dependency>,
}

impl Manifest {
    /// A manifest for a project that declares nothing.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            entry_points: Vec::new(),
            dependencies: Vec::new(),
        }
    }

    /// One manifest holding every entry point and dependency of `manifests`.
    #[must_use]
    pub fn merged<'a>(manifests: impl IntoIterator<Item = &'a Self>) -> Self {
        let mut merged = Self::empty();
        for manifest in manifests {
            merged
                .entry_points
                .extend(manifest.entry_points.iter().cloned());
            merged
                .dependencies
                .extend(manifest.dependencies.iter().cloned());
        }
        merged
    }

    /// Whether `module:name` is a declared entry point.
    #[must_use]
    pub fn declares_entry_point(&self, module: &ModulePath, name: &SymbolName) -> bool {
        self.entry_points
            .iter()
            .any(|ep| &ep.module == module && ep.attribute.as_ref() == Some(name))
    }
}

#[cfg(test)]
mod tests {
    use super::DistributionName;

    #[test]
    fn normalises_like_pep_503() {
        assert_eq!(DistributionName::normalize("Pillow").as_str(), "pillow");
        assert_eq!(
            DistributionName::normalize("python_dateutil").as_str(),
            "python-dateutil"
        );
        assert_eq!(
            DistributionName::normalize("zope.interface").as_str(),
            "zope-interface"
        );
        assert_eq!(DistributionName::normalize("A--b__c").as_str(), "a-b-c");
    }

    #[test]
    fn conventional_module_uses_underscores() {
        assert_eq!(
            DistributionName::normalize("typing-extensions").conventional_module(),
            "typing_extensions"
        );
    }
}
