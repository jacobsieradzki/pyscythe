//! Conventions of Python itself that make a definition reachable without a
//! reference: typing overload stubs.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::plugins::decorated_with_from;

pub(crate) struct Python;

impl KeepRule for Python {
    fn plugin(&self) -> PluginName {
        PluginName::Python
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        decorated_with_from(
            context.symbol,
            &["overload"],
            false,
            &["typing", "typing_extensions"],
        )
        .then_some("typing overload stub")
    }
}

#[cfg(test)]
mod tests {
    use super::Python;
    use crate::plugins::testing::Case;

    #[test]
    fn keeps_overload_stubs() {
        assert!(
            Case::function("f")
                .decorated("overload")
                .is_kept_by(&Python)
        );
        assert!(
            Case::method("f")
                .decorated("typing.overload")
                .is_kept_by(&Python)
        );
        assert!(!Case::function("f").is_kept_by(&Python));
    }
}
