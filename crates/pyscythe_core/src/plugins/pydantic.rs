//! Pydantic calls validators and serializers itself; they are never referenced.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::plugins::decorated_with_from;

pub(crate) struct Pydantic;

/// Model base classes by convention, for a project whose dependencies are not
/// installed: plain Pydantic, its settings package, and `SQLModel`.
const MODEL_BASES: &[&str] = &["BaseModel", "BaseSettings", "SQLModel"];

const DECORATORS: &[&str] = &[
    "field_validator",
    "model_validator",
    "validator",
    "root_validator",
    "field_serializer",
    "model_serializer",
    "computed_field",
];

impl KeepRule for Pydantic {
    fn plugin(&self) -> PluginName {
        PluginName::Pydantic
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        if decorated_with_from(context.symbol, DECORATORS, false, &["pydantic"]) {
            return Some("invoked by Pydantic");
        }
        if context.attribute_of_class_from("pydantic", MODEL_BASES)
            || context.attribute_of_class_from("pydantic_settings", MODEL_BASES)
            || context.attribute_of_class_from("sqlmodel", MODEL_BASES)
        {
            return Some("a field of a Pydantic model, which is its schema");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Pydantic;
    use crate::plugins::testing::Case;

    #[test]
    fn keeps_validators_and_serializers() {
        assert!(
            Case::function("check")
                .decorated("field_validator")
                .is_kept_by(&Pydantic)
        );
        assert!(
            Case::function("check")
                .decorated("pydantic.model_validator")
                .is_kept_by(&Pydantic)
        );
        assert!(!Case::function("check").is_kept_by(&Pydantic));
    }

    #[test]
    fn keeps_model_fields_whether_or_not_the_bases_resolved() {
        assert!(
            Case::attribute("name")
                .on_class(Case::class("Event").with_ancestors(&["pydantic.main.BaseModel"]))
                .is_kept_by(&Pydantic)
        );
        assert!(
            Case::attribute("name")
                .on_class(Case::class("Event").extending("SQLModel"))
                .is_kept_by(&Pydantic),
            "a project without its dependencies installed still has a schema"
        );
        assert!(
            !Case::attribute("name")
                .on_class(Case::class("Event"))
                .is_kept_by(&Pydantic)
        );
    }
}
