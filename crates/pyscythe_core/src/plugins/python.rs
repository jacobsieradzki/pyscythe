//! Conventions of Python itself that make a definition reachable without a
//! reference: typing overload stubs, names exported through `__all__`, and
//! the public names of modules configured as the project's API.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::plugins::decorated_with_from;

pub(crate) struct Python;

impl KeepRule for Python {
    fn plugin(&self) -> PluginName {
        PluginName::Python
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        let symbol = context.symbol;
        if decorated_with_from(
            symbol,
            &["overload"],
            false,
            &["typing", "typing_extensions"],
        ) {
            return Some("typing overload stub");
        }
        if symbol.is_module_level() && context.file.exports.contains(&symbol.name) {
            return Some("listed in __all__");
        }
        let in_public_module = context.file.module.as_ref().is_some_and(|module| {
            context
                .public_modules
                .iter()
                .any(|prefix| prefix.covers(module.as_str()))
        });
        if symbol.is_module_level() && !symbol.name.is_private() && in_public_module {
            return Some("public name of a module configured as API");
        }
        if context.file_name() == "conf.py"
            && context.is_under_directory_starting_with("doc")
            && symbol.is_module_level()
        {
            return Some("Sphinx configuration read by name");
        }
        if context.file_role == crate::keep::FileRole::ToolConfig && symbol.is_module_level() {
            return Some("configuration script read by the tool that runs it");
        }
        if context.file_role == crate::keep::FileRole::LoadedByPath && symbol.is_module_level() {
            return Some("script that a string names by path");
        }
        if crate::dead_code::is_in_root_directory(context.file) {
            return Some("in a directory of scripts, examples, docs, or benchmarks");
        }
        if crate::dead_code::is_test_data_file(context.file) {
            return Some("test data or helper script under the tests tree");
        }
        if crate::dead_code::is_typing_test_file(context.file) {
            return Some("read by a type checker rather than run");
        }
        if crate::dead_code::is_vendored_file(context.file) {
            return Some("vendored third-party code, updated by re-copying it");
        }
        if symbol.kind == crate::symbol::SymbolKind::Method
            && symbol.name.as_str().starts_with("do_")
            && context.ancestry.has_ancestor_in("http.server")
        {
            return Some("request handler dispatched by HTTP method name");
        }
        if symbol.kind == crate::symbol::SymbolKind::Class
            && context.registration == crate::index::SubclassRegistration::ByBaseHook
        {
            return Some("registered by a base class __init_subclass__ hook");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Python;
    use crate::plugins::testing::Case;

    #[test]
    fn keeps_files_a_type_checker_reads_but_not_a_source_module_named_typing() {
        assert!(
            Case::function("accepts_a_string")
                .at("/p/tests/typing/check_core.py")
                .is_kept_by(&Python)
        );
        assert!(
            Case::function("returns_a_string")
                .at("/p/typing_tests/baseline.py")
                .is_kept_by(&Python)
        );
        assert!(
            !Case::function("forgotten")
                .at("/p/pkg/typing.py")
                .is_kept_by(&Python),
            "a source module named typing is ordinary code"
        );
    }

    #[test]
    fn keeps_everything_in_example_and_benchmark_directories() {
        assert!(
            Case::variable("console")
                .at("/p/examples/demo.py")
                .is_kept_by(&Python)
        );
        assert!(
            Case::method("time_wrap")
                .at("/p/benchmarks/bench.py")
                .is_kept_by(&Python)
        );
        assert!(
            !Case::variable("console")
                .at("/p/pkg/demo.py")
                .is_kept_by(&Python)
        );
    }

    #[test]
    fn keeps_classes_registered_by_a_base_hook() {
        assert!(
            Case::class("PluginA")
                .registered_by_base()
                .is_kept_by(&Python)
        );
        assert!(!Case::class("Plain").is_kept_by(&Python));
    }

    #[test]
    fn keeps_module_level_names_of_tool_configuration_scripts() {
        assert!(Case::variable("bind").in_tool_config().is_kept_by(&Python));
        assert!(!Case::variable("bind").is_kept_by(&Python));
    }

    #[test]
    fn keeps_test_data_and_helper_scripts_under_a_tests_tree() {
        assert!(
            Case::function("load")
                .at("/p/test/mitmproxy/data/addonscripts/addon.py")
                .is_kept_by(&Python)
        );
        assert!(
            !Case::function("load")
                .at("/p/pkg/data/loader.py")
                .is_kept_by(&Python)
        );
    }

    #[test]
    fn keeps_vendored_third_party_trees() {
        assert!(
            Case::function("connect")
                .at("/p/pkg/_vendor/tinylib/api.py")
                .is_kept_by(&Python)
        );
        assert!(
            Case::function("helper")
                .at("/p/pkg/vendor/bundled.py")
                .is_kept_by(&Python)
        );
        assert!(
            !Case::function("start")
                .at("/p/pkg/core/engine.py")
                .is_kept_by(&Python)
        );
    }

    #[test]
    fn keeps_scenarios_an_integration_harness_copies_into_place() {
        assert!(
            Case::function("main")
                .at("/p/test/integration/targets/demo/library/mod.py")
                .is_kept_by(&Python)
        );
        assert!(
            Case::function("assist")
                .at("/p/test/support/helper/plugins/thing.py")
                .is_kept_by(&Python)
        );
        assert!(
            !Case::function("assist")
                .at("/p/pkg/support/thing.py")
                .is_kept_by(&Python),
            "a support directory in the source tree is ordinary code"
        );
    }

    #[test]
    fn keeps_http_method_handlers_on_request_handler_subclasses() {
        assert!(
            Case::method("do_GET")
                .with_ancestors(&["http.server.BaseHTTPRequestHandler"])
                .is_kept_by(&Python)
        );
        assert!(!Case::method("do_GET").is_kept_by(&Python));
        assert!(
            !Case::method("helper")
                .with_ancestors(&["http.server.BaseHTTPRequestHandler"])
                .is_kept_by(&Python)
        );
    }

    #[test]
    fn keeps_module_level_names_of_scripts_named_by_path() {
        assert!(
            Case::function("main")
                .in_role(crate::keep::FileRole::LoadedByPath)
                .is_kept_by(&Python)
        );
        assert!(
            !Case::method("helper")
                .in_role(crate::keep::FileRole::LoadedByPath)
                .is_kept_by(&Python)
        );
    }

    #[test]
    fn keeps_sphinx_configuration() {
        assert!(
            Case::variable("project")
                .at("/p/docs/conf.py")
                .is_kept_by(&Python)
        );
        assert!(
            !Case::variable("project")
                .at("/p/pkg/conf.py")
                .is_kept_by(&Python)
        );
    }

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
