//! Ansible finds action plugins, lookups, filters, and modules by name
//! through its plugin loader, and ships modules to run on the target host.
//! Nothing in the tree imports any of them, so they read as dead files.
//!
//! The same layout is what every Ansible collection uses, so this covers a
//! collection repository as well as ansible-core itself.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::source::SourceFile;

pub(crate) struct Ansible;

/// The plugin kinds the loader knows, each a directory under `plugins`.
const PLUGIN_KINDS: &[&str] = &[
    "action",
    "become",
    "cache",
    "callback",
    "cliconf",
    "connection",
    "doc_fragments",
    "filter",
    "httpapi",
    "inventory",
    "lookup",
    "module_utils",
    "modules",
    "netconf",
    "shell",
    "strategy",
    "terminal",
    "test",
    "vars",
];

/// Whether the file sits in a tree the plugin loader searches: a `plugins`
/// directory holding one of the known kinds, or `ansible/modules` itself.
fn is_loaded_by_name(file: &SourceFile) -> bool {
    let components: Vec<&str> = file
        .relative_path
        .components()
        .map(|component| component.as_str())
        .collect();
    components.windows(2).any(|pair| match pair {
        [parent, kind] => {
            (*parent == "plugins" && PLUGIN_KINDS.contains(kind))
                || (*parent == "ansible" && *kind == "modules")
        }
        _ => false,
    })
}

impl KeepRule for Ansible {
    fn plugin(&self) -> PluginName {
        PluginName::Ansible
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        if context.symbol.is_module_level() && is_loaded_by_name(context.file) {
            return Some("found by name by the Ansible plugin loader");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Ansible;
    use crate::plugins::testing::Case;

    #[test]
    fn keeps_plugins_and_modules_the_loader_finds() {
        assert!(
            Case::class("LookupModule")
                .at("/p/lib/ansible/plugins/lookup/env.py")
                .is_kept_by(&Ansible)
        );
        assert!(
            Case::function("main")
                .at("/p/lib/ansible/modules/ping.py")
                .is_kept_by(&Ansible)
        );
        assert!(
            Case::function("main")
                .at("/p/my_collection/plugins/modules/thing.py")
                .is_kept_by(&Ansible),
            "a collection uses the same layout"
        );
    }

    #[test]
    fn leaves_ordinary_library_code_alone() {
        assert!(
            !Case::function("forgotten_helper")
                .at("/p/lib/ansible/utils/helpers.py")
                .is_kept_by(&Ansible)
        );
        assert!(
            !Case::function("render")
                .at("/p/app/plugins/rendering/html.py")
                .is_kept_by(&Ansible),
            "`rendering` is not a kind the loader knows"
        );
    }
}
