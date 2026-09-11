//! Flask registers views and hooks through `app.route`, blueprint decorators,
//! and a family of request lifecycle decorators.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::plugins::decorated_with_from;
use crate::symbol::SymbolKind;

pub(crate) struct Flask;

/// `MethodView` dispatches to these by request method.
const VIEW_METHODS: &[&str] = &[
    "get",
    "post",
    "put",
    "patch",
    "delete",
    "head",
    "options",
    "dispatch_request",
];

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
        let symbol = context.symbol;
        if decorated_with_from(symbol, DECORATORS, true, &["flask"]) {
            return Some("registered with a Flask app or blueprint");
        }
        if symbol.kind == SymbolKind::Method && VIEW_METHODS.contains(&symbol.name.as_str()) {
            return Some("Flask view method dispatched by HTTP verb");
        }
        None
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
    fn keeps_method_view_verbs() {
        assert!(Case::method("get").is_kept_by(&Flask));
        assert!(Case::method("dispatch_request").is_kept_by(&Flask));
        assert!(!Case::method("render").is_kept_by(&Flask));
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
