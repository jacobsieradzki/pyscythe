//! Symbols named by `[project.scripts]` and `[project.entry-points]`.

use crate::keep::{KeepContext, KeepRule, PluginName};

pub(crate) struct EntryPoints;

impl KeepRule for EntryPoints {
    fn plugin(&self) -> PluginName {
        PluginName::EntryPoints
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        let module = context.file.module.as_ref()?;
        context
            .manifest
            .declares_entry_point(module, &context.symbol.name)
            .then_some("declared as an entry point in pyproject.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::EntryPoints;
    use crate::manifest::{EntryPoint, EntryPointKind, Manifest};
    use crate::plugins::testing::Case;
    use crate::source::ModulePath;
    use crate::symbol::SymbolName;

    fn manifest_with(module: &str, attribute: &str) -> Manifest {
        Manifest {
            entry_points: vec![EntryPoint {
                kind: EntryPointKind::Script,
                module: ModulePath::new(module),
                attribute: Some(SymbolName::new(attribute)),
            }],
            dependencies: Vec::new(),
        }
    }

    #[test]
    fn keeps_the_function_a_script_points_at() {
        let case = Case::function("main")
            .in_module("pkg.cli")
            .with_manifest(manifest_with("pkg.cli", "main"));
        assert!(case.is_kept_by(&EntryPoints));
    }

    #[test]
    fn does_not_keep_a_same_named_function_in_another_module() {
        let case = Case::function("main")
            .in_module("pkg.other")
            .with_manifest(manifest_with("pkg.cli", "main"));
        assert!(!case.is_kept_by(&EntryPoints));
    }

    #[test]
    fn does_nothing_without_entry_points() {
        assert!(!Case::function("main").is_kept_by(&EntryPoints));
    }
}
