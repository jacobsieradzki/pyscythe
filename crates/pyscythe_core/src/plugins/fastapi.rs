//! `FastAPI` registers handlers through `app.<verb>` and `router.<verb>` decorators.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::plugins::decorated_with;

pub(crate) struct FastApi;

const ROUTE_DECORATORS: &[&str] = &[
    "get",
    "post",
    "put",
    "patch",
    "delete",
    "options",
    "head",
    "trace",
    "api_route",
    "websocket",
    "websocket_route",
];

const HOOK_DECORATORS: &[&str] = &["on_event", "exception_handler", "middleware"];

impl KeepRule for FastApi {
    fn plugin(&self) -> PluginName {
        PluginName::FastApi
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        if decorated_with(context.symbol, ROUTE_DECORATORS, true) {
            return Some("registered as a route handler");
        }
        if decorated_with(context.symbol, HOOK_DECORATORS, true) {
            return Some("registered as an application hook");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::FastApi;
    use crate::plugins::testing::Case;

    #[test]
    fn keeps_route_handlers() {
        assert!(
            Case::function("get_user")
                .decorated("router.get")
                .is_kept_by(&FastApi)
        );
        assert!(
            Case::function("login")
                .decorated("app.post")
                .is_kept_by(&FastApi)
        );
        assert!(
            Case::function("ws")
                .decorated("router.websocket")
                .is_kept_by(&FastApi)
        );
    }

    #[test]
    fn keeps_lifecycle_hooks() {
        assert!(
            Case::function("startup")
                .decorated("app.on_event")
                .is_kept_by(&FastApi)
        );
        assert!(
            Case::function("handle")
                .decorated("app.exception_handler")
                .is_kept_by(&FastApi)
        );
    }

    #[test]
    fn a_bare_decorator_called_get_is_not_a_route() {
        assert!(
            !Case::function("thing")
                .decorated("get")
                .is_kept_by(&FastApi)
        );
    }

    #[test]
    fn undecorated_functions_are_not_kept() {
        assert!(!Case::function("helper").is_kept_by(&FastApi));
    }
}
