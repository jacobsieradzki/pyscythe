//! Celery discovers tasks through decorators and wires signals with `.connect`.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::plugins::decorated_with;
use crate::symbol::SymbolKind;

pub(crate) struct Celery;

/// Methods on `Task` subclasses that the worker calls.
const TASK_METHODS: &[&str] = &[
    "run",
    "before_start",
    "on_success",
    "on_failure",
    "on_retry",
    "after_return",
];

const TASK_WITH_RECEIVER: &[&str] = &["task", "periodic_task", "shared_task"];
const TASK_BARE: &[&str] = &["shared_task", "periodic_task"];

impl KeepRule for Celery {
    fn plugin(&self) -> PluginName {
        PluginName::Celery
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        let symbol = context.symbol;
        if decorated_with(symbol, TASK_WITH_RECEIVER, true)
            || decorated_with(symbol, TASK_BARE, false)
        {
            return Some("registered as a Celery task");
        }
        if decorated_with(symbol, &["connect"], true) {
            return Some("connected to a signal");
        }
        if symbol.kind == SymbolKind::Method && TASK_METHODS.contains(&symbol.name.as_str()) {
            return Some("Celery task hook called by name");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Celery;
    use crate::plugins::testing::Case;

    #[test]
    fn keeps_tasks_and_signal_handlers() {
        assert!(
            Case::function("send")
                .decorated("app.task")
                .is_kept_by(&Celery)
        );
        assert!(
            Case::function("send")
                .decorated("shared_task")
                .is_kept_by(&Celery)
        );
        assert!(
            Case::function("send")
                .decorated("celery.shared_task")
                .is_kept_by(&Celery)
        );
        assert!(
            Case::function("on_ready")
                .decorated("worker_ready.connect")
                .is_kept_by(&Celery)
        );
    }

    #[test]
    fn keeps_task_class_hooks() {
        assert!(Case::method("run").is_kept_by(&Celery));
        assert!(Case::method("on_failure").is_kept_by(&Celery));
        assert!(!Case::method("helper").is_kept_by(&Celery));
    }

    #[test]
    fn a_bare_task_decorator_belongs_to_another_plugin() {
        assert!(!Case::function("t").decorated("task").is_kept_by(&Celery));
    }
}
