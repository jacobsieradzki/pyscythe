//! Alembic executes migration scripts by loading well-known module attributes.

use crate::keep::{FileRole, KeepContext, KeepRule, PluginName};

pub(crate) struct Alembic;

const SCRIPT_ATTRIBUTES: &[&str] = &[
    "revision",
    "down_revision",
    "branch_labels",
    "depends_on",
    "upgrade",
    "downgrade",
];

impl KeepRule for Alembic {
    fn plugin(&self) -> PluginName {
        PluginName::Alembic
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        let name = context.symbol.name.as_str();
        if context.parent_directory_is("versions") && SCRIPT_ATTRIBUTES.contains(&name) {
            return Some("Alembic migration script attribute");
        }
        if context.file_name() == "env.py" && context.is_under_directory("alembic") {
            return Some("Alembic environment script");
        }
        if context.file_role == FileRole::AlembicScript && context.symbol.is_module_level() {
            return Some("Alembic script loaded by path");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Alembic;
    use crate::plugins::testing::Case;

    #[test]
    fn keeps_migration_script_attributes() {
        assert!(
            Case::function("upgrade")
                .at("/p/alembic/versions/abc_init.py")
                .is_kept_by(&Alembic)
        );
        assert!(
            Case::variable("down_revision")
                .at("/p/migrations/versions/abc.py")
                .is_kept_by(&Alembic)
        );
        assert!(
            !Case::function("helper")
                .at("/p/alembic/versions/abc.py")
                .is_kept_by(&Alembic)
        );
    }

    #[test]
    fn keeps_module_level_names_of_scripts_alembic_loads_by_path() {
        assert!(
            Case::function("show_secrets_encoder")
                .at("/p/db/revisions/versions/2021_abc.py")
                .in_alembic_script()
                .is_kept_by(&Alembic)
        );
        assert!(
            !Case::function("show_secrets_encoder")
                .at("/p/db/revisions/versions/2021_abc.py")
                .is_kept_by(&Alembic)
        );
    }

    #[test]
    fn keeps_env_script() {
        assert!(
            Case::function("run_migrations_online")
                .at("/p/alembic/env.py")
                .is_kept_by(&Alembic)
        );
        assert!(
            !Case::function("run")
                .at("/p/pkg/env.py")
                .is_kept_by(&Alembic)
        );
    }
}
