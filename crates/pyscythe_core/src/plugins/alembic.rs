//! Alembic executes migration scripts by loading well-known module attributes.

use crate::keep::{KeepContext, KeepRule, PluginName};

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
