//! Home Assistant loads integrations by name: `homeassistant.components.<domain>`
//! and `custom_components.<domain>`, their platform modules (`sensor.py`,
//! `config_flow.py`, `diagnostics.py`), the setup hooks it calls on each, the
//! schema constants it reads, and config-flow steps dispatched by step id.

use crate::keep::{FileRole, KeepContext, KeepRule, PluginName};
use crate::symbol::SymbolKind;

pub(crate) struct HomeAssistant;

/// Module-level functions Home Assistant calls on an integration or platform.
const HOOKS: &[&str] = &[
    "setup",
    "async_setup",
    "async_setup_entry",
    "async_unload_entry",
    "async_migrate_entry",
    "async_remove_entry",
    "async_remove_config_entry_device",
    "setup_platform",
    "async_setup_platform",
    "async_setup_scanner",
    "async_get_config_entry_diagnostics",
    "async_get_device_diagnostics",
    "async_get_triggers",
    "async_get_conditions",
    "async_get_actions",
    "async_attach_trigger",
    "async_validate_trigger_config",
    "async_validate_condition_config",
    "async_validate_action_config",
    "async_call_action_from_config",
    "async_condition_from_config",
    "async_get_trigger_capabilities",
    "async_get_condition_capabilities",
    "async_get_action_capabilities",
    "async_get_service",
    "async_get_backup_agents",
    "async_register_backup_agents_listener",
    "async_setup_intents",
    "async_get_options_flow",
    "async_get_media_source",
    "async_browse_media",
    "async_describe_events",
    "async_describe_on_off_states",
    "async_get_significant_states",
    "async_setup_services",
];

/// Module-level names Home Assistant reads from an integration or platform.
const CONSTANTS: &[&str] = &[
    "DOMAIN",
    "PLATFORMS",
    "CONFIG_SCHEMA",
    "PLATFORM_SCHEMA",
    "PLATFORM_SCHEMA_BASE",
    "TRIGGER_SCHEMA",
    "CONDITION_SCHEMA",
    "ACTION_SCHEMA",
    "PARALLEL_UPDATES",
    "SCAN_INTERVAL",
    "ENTITY_ID_FORMAT",
    "SERVICE_SCHEMA",
    "DEFAULT_NAME",
];

impl KeepRule for HomeAssistant {
    fn plugin(&self) -> PluginName {
        PluginName::HomeAssistant
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        if context.file_role != FileRole::HomeAssistantIntegration {
            return None;
        }
        let symbol = context.symbol;
        let name = symbol.name.as_str();
        if symbol.is_module_level() && HOOKS.contains(&name) {
            return Some("Home Assistant setup hook called by name");
        }
        if symbol.is_module_level() && CONSTANTS.contains(&name) {
            return Some("Home Assistant integration constant read by name");
        }
        if symbol.kind == SymbolKind::Method && name.starts_with("async_step_") {
            return Some("Home Assistant config flow step dispatched by step id");
        }
        if symbol.kind == SymbolKind::Class && context.file_name() == "config_flow.py" {
            return Some("Home Assistant config flow handler registered by domain");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::HomeAssistant;
    use crate::keep::FileRole;
    use crate::plugins::testing::Case;

    #[test]
    fn keeps_hooks_constants_and_flow_steps_of_integrations_only() {
        let integration = |case: Case| case.in_role(FileRole::HomeAssistantIntegration);
        assert!(integration(Case::function("async_setup_entry")).is_kept_by(&HomeAssistant));
        assert!(integration(Case::variable("PARALLEL_UPDATES")).is_kept_by(&HomeAssistant));
        assert!(integration(Case::method("async_step_user")).is_kept_by(&HomeAssistant));
        assert!(
            integration(
                Case::class("ToloConfigFlow").at("/p/custom_components/tolo/config_flow.py")
            )
            .is_kept_by(&HomeAssistant)
        );
        assert!(!integration(Case::function("build_url")).is_kept_by(&HomeAssistant));
        assert!(!Case::function("async_setup_entry").is_kept_by(&HomeAssistant));
    }
}
