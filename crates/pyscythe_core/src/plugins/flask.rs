//! Flask registers views and hooks through `app.route`, blueprint decorators,
//! and a family of request lifecycle decorators.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::plugins::decorated_with;

pub(crate) struct Flask;

const DECORATORS: &[&str] = &[
    "route",
    "get",
    "post",
    "put",
    "patch",
    "delete",
    "errorhandler",
    "before_request",
    "after_request",
    "teardown_request",
    "teardown_appcontext",
    "before_app_request",
    "after_app_request",
    "context_processor",
    "app_context_processor",
    "template_filter",
    "app_template_filter",
    "template_global",
    "app_template_global",
    "template_test",
    "url_value_preprocessor",
    "url_defaults",
    "shell_context_processor",
    "record",
    "record_once",
];

impl KeepRule for Flask {
    fn plugin(&self) -> PluginName {
        PluginName::Flask
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        decorated_with(context.symbol, DECORATORS, true)
            .then_some("registered with a Flask app or blueprint")
    }
}

#[cfg(test)]
mod tests {
    use super::Flask;
    use crate::plugins::testing::Case;

    #[test]
    fn keeps_routes_and_hooks() {
        assert!(
            Case::function("index")
                .decorated("app.route")
                .is_kept_by(&Flask)
        );
        assert!(
            Case::function("index")
                .decorated("bp.get")
                .is_kept_by(&Flask)
        );
        assert!(
            Case::function("boom")
                .decorated("app.errorhandler")
                .is_kept_by(&Flask)
        );
        assert!(
            Case::function("inject")
                .decorated("app.context_processor")
                .is_kept_by(&Flask)
        );
    }

    #[test]
    fn ignores_unrelated_decorators() {
        assert!(
            !Case::function("f")
                .decorated("functools.cache")
                .is_kept_by(&Flask)
        );
    }
}
