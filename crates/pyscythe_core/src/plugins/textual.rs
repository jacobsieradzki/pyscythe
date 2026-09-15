//! Textual dispatches a message by building a handler name from the message
//! class: `Click` becomes `on_click`, and the pump looks for `_on_click`
//! first. Nothing in the project ever names those methods, so they read as
//! dead.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::symbol::SymbolKind;

pub(crate) struct Textual;

/// The class every widget, screen, and app descends from.
const MESSAGE_PUMP: &str = "textual.message_pump";

/// Base names that mean a message pump when ty could not resolve the
/// ancestry, which is what happens without textual installed.
fn looks_like_message_pump(base: &str) -> bool {
    matches!(
        base,
        "MessagePump" | "DOMNode" | "Widget" | "App" | "Screen" | "ScrollView" | "ModalScreen"
    )
}

fn is_message_pump(context: &KeepContext<'_>) -> bool {
    if context.ancestry.has_ancestor_in(MESSAGE_PUMP) {
        return true;
    }
    !context.ancestry.is_complete()
        && context
            .ancestry
            .names()
            .iter()
            .any(|base| looks_like_message_pump(base.last_segment()))
}

impl KeepRule for Textual {
    fn plugin(&self) -> PluginName {
        PluginName::Textual
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        let symbol = context.symbol;
        if symbol.kind != SymbolKind::Method {
            return None;
        }
        let name = symbol.name.as_str();
        let is_handler = name.starts_with("on_") || name.starts_with("_on_");
        if is_handler && is_message_pump(&context) {
            return Some("Textual message handler named from the message class");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Textual;
    use crate::plugins::testing::Case;

    #[test]
    fn keeps_handlers_on_message_pumps() {
        assert!(
            Case::method("on_click")
                .with_ancestors(&["textual.message_pump.MessagePump"])
                .is_kept_by(&Textual)
        );
        assert!(
            Case::method("_on_mount")
                .with_ancestors(&["textual.message_pump.MessagePump"])
                .is_kept_by(&Textual)
        );
        assert!(
            Case::method("_on_mount")
                .with_unresolved_ancestors(&["Widget"])
                .is_kept_by(&Textual)
        );
    }

    #[test]
    fn leaves_other_methods_and_other_classes_alone() {
        assert!(
            !Case::method("helper")
                .with_ancestors(&["textual.message_pump.MessagePump"])
                .is_kept_by(&Textual)
        );
        assert!(
            !Case::method("_on_mount")
                .with_ancestors(&["app.models.Record"])
                .is_kept_by(&Textual),
            "a handler-shaped name on an ordinary class is not dispatched to"
        );
    }
}
