//! `SQLAlchemy` registers table models with metadata when the class statement
//! runs, and invokes event listeners registered through `event.listens_for`.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::plugins::decorated_with;
use crate::symbol::SymbolKind;

pub(crate) struct SqlAlchemy;

/// Decorators that hand a method to the ORM.
const METHOD_DECORATORS: &[&str] = &[
    "validates",
    "hybrid_property",
    "declared_attr",
    "reconstructor",
];

/// Declarative bases by convention: plain `SQLAlchemy`, Flask-`SQLAlchemy`, and `SQLModel`.
const DECLARATIVE_BASES: &[&str] = &["Base", "DeclarativeBase", "Model"];

impl KeepRule for SqlAlchemy {
    fn plugin(&self) -> PluginName {
        PluginName::SqlAlchemy
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        let symbol = context.symbol;
        if symbol.kind == SymbolKind::Class {
            if symbol.has_class_keyword("table") {
                return Some("SQLModel table registered with metadata");
            }
            if context.descends_from("sqlalchemy", DECLARATIVE_BASES)
                || context.descends_from("flask_sqlalchemy", DECLARATIVE_BASES)
            {
                return Some("ORM model registered with declarative metadata");
            }
        }
        if decorated_with(symbol, &["listens_for"], true)
            || decorated_with(symbol, &["listens_for"], false)
        {
            return Some("registered as a SQLAlchemy event listener");
        }
        if decorated_with(symbol, METHOD_DECORATORS, false) {
            return Some("SQLAlchemy mapped attribute or validator");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::SqlAlchemy;
    use crate::plugins::testing::Case;

    #[test]
    fn keeps_event_listeners() {
        assert!(
            Case::function("on_connect")
                .decorated("event.listens_for")
                .is_kept_by(&SqlAlchemy)
        );
        assert!(
            Case::function("on_connect")
                .decorated("listens_for")
                .is_kept_by(&SqlAlchemy)
        );
        assert!(!Case::function("query").is_kept_by(&SqlAlchemy));
    }

    #[test]
    fn keeps_orm_method_decorators() {
        assert!(
            Case::method("check")
                .decorated("validates")
                .is_kept_by(&SqlAlchemy)
        );
        assert!(
            Case::method("full")
                .decorated("hybrid_property")
                .is_kept_by(&SqlAlchemy)
        );
    }

    #[test]
    fn keeps_table_models() {
        assert!(
            Case::class("Event")
                .with_class_keyword("table")
                .is_kept_by(&SqlAlchemy)
        );
        assert!(
            Case::class("User")
                .extending("Base")
                .is_kept_by(&SqlAlchemy)
        );
        assert!(
            Case::class("User")
                .extending("db.Model")
                .is_kept_by(&SqlAlchemy)
        );
        assert!(
            Case::class("User")
                .extending("DeclarativeBase")
                .is_kept_by(&SqlAlchemy)
        );
    }

    #[test]
    fn resolved_ancestry_beats_base_names() {
        let orm = Case::class("User").extending("Base").with_ancestors(&[
            "pkg.db.Base",
            "sqlalchemy.orm.decl_api.DeclarativeBase",
            "builtins.object",
        ]);
        assert!(orm.is_kept_by(&SqlAlchemy));

        let local = Case::class("Child")
            .extending("Base")
            .with_ancestors(&["pkg.widget.Base", "builtins.object"]);
        assert!(
            !local.is_kept_by(&SqlAlchemy),
            "a local class called Base is not an ORM base"
        );
    }

    #[test]
    fn plain_classes_and_non_table_sqlmodels_are_not_kept() {
        assert!(!Case::class("Service").is_kept_by(&SqlAlchemy));
        assert!(
            !Case::class("EventRead")
                .extending("SQLModel")
                .is_kept_by(&SqlAlchemy)
        );
        assert!(
            !Case::class("Row")
                .extending("BaseModel")
                .is_kept_by(&SqlAlchemy)
        );
    }
}
