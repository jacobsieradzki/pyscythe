//! Click and Typer register commands by decorating functions.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::plugins::decorated_with_from;

const PACKAGES: &[&str] = &["click", "typer"];

pub(crate) struct Click;

const WITH_RECEIVER: &[&str] = &["command", "group", "callback"];
const BARE: &[&str] = &["command", "group"];

impl KeepRule for Click {
    fn plugin(&self) -> PluginName {
        PluginName::Click
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        (decorated_with_from(context.symbol, WITH_RECEIVER, true, PACKAGES)
            || decorated_with_from(context.symbol, BARE, false, PACKAGES))
        .then_some("registered as a CLI command")
    }
}

#[cfg(test)]
mod tests {
    use super::Click;
    use crate::plugins::testing::Case;

    #[test]
    fn keeps_click_and_typer_commands() {
        assert!(
            Case::function("run")
                .decorated("cli.command")
                .is_kept_by(&Click)
        );
        assert!(
            Case::function("run")
                .decorated("click.command")
                .is_kept_by(&Click)
        );
        assert!(
            Case::function("run")
                .decorated("command")
                .is_kept_by(&Click)
        );
        assert!(
            Case::function("root")
                .decorated("app.callback")
                .is_kept_by(&Click)
        );
        assert!(
            Case::function("grp")
                .decorated("click.group")
                .is_kept_by(&Click)
        );
    }

    #[test]
    fn a_bare_callback_is_not_a_command() {
        assert!(
            !Case::function("cb")
                .decorated("callback")
                .is_kept_by(&Click)
        );
    }
}
