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

/// Facts from `pyproject.toml` and friends that analyses need.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// Every declared entry point.
    pub entry_points: Vec<EntryPoint>,
}

impl Manifest {
    /// A manifest for a project that declares nothing.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            entry_points: Vec::new(),
        }
    }

    /// Whether `module:name` is a declared entry point.
    #[must_use]
    pub fn declares_entry_point(&self, module: &ModulePath, name: &SymbolName) -> bool {
        self.entry_points
            .iter()
            .any(|ep| &ep.module == module && ep.attribute.as_ref() == Some(name))
    }
}
