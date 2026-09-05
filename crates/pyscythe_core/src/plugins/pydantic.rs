//! Pydantic calls validators and serializers itself; they are never referenced.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::plugins::decorated_with;

pub(crate) struct Pydantic;

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
        decorated_with(context.symbol, DECORATORS, false).then_some("invoked by Pydantic")
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
}
