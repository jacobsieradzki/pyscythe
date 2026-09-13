//! pytest collects tests by name, loads `conftest.py` implicitly, and runs
//! `autouse` fixtures without anyone naming them.

use crate::index::NameUsage;
use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::plugins::decorated_with_from;

const PACKAGES: &[&str] = &["pytest", "_pytest"];
use crate::symbol::SymbolKind;

pub(crate) struct Pytest;

/// pytest and unittest call these by name.
const TEST_LIFECYCLE_METHODS: &[&str] = &[
    "setup_method",
    "teardown_method",
    "setup_class",
    "teardown_class",
    "setup",
    "teardown",
    "setUp",
    "tearDown",
    "setUpClass",
    "tearDownClass",
    "setUpTestData",
    "runTest",
];

/// Fixtures that pytest plugins define and projects override by name.
const PLUGIN_FIXTURES: &[&str] = &[
    "event_loop",
    "event_loop_policy",
    "anyio_backend",
    "celery_config",
    "celery_app",
    "celery_worker_parameters",
    "celery_includes",
    "django_db_setup",
    "django_db_modify_db_settings",
];

impl KeepRule for Pytest {
    fn plugin(&self) -> PluginName {
        PluginName::Pytest
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        let symbol = context.symbol;
        let name = symbol.name.as_str();
        let file_name = context.file_name();

        if file_name == "conftest.py" {
            return Some("conftest.py is loaded by pytest");
        }
        if context.is_test_file() {
            if matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method)
                && context.tests.collects_function(name)
            {
                return Some("collected as a test");
            }
            if symbol.kind == SymbolKind::Class && context.tests.collects_class(name) {
                return Some("collected as a test class");
            }
            if symbol.kind == SymbolKind::Method && TEST_LIFECYCLE_METHODS.contains(&name) {
                return Some("test lifecycle hook");
            }
        }
        // unittest and Django run `test*` methods and the lifecycle hooks of
        // any TestCase subclass, wherever the file lives.
        if symbol.kind == SymbolKind::Method
            && (context.ancestry.has_ancestor_in("unittest")
                || context.ancestry.has_ancestor_in("django.test"))
            && (context.tests.collects_function(name) || TEST_LIFECYCLE_METHODS.contains(&name))
        {
            return Some("test method of a TestCase subclass");
        }
        if symbol.is_module_level() && name == "__unittest" {
            return Some("unittest traceback marker read by name");
        }
        if symbol.kind == SymbolKind::Function && name.starts_with("pytest_") {
            return Some("pytest hook");
        }
        if symbol.has_decorator(|d| d.name.last_segment() == "fixture" && d.has_keyword("autouse"))
        {
            return Some("autouse fixture");
        }
        if symbol.has_decorator(|d| d.name.last_segment() == "fixture" && d.has_keyword("name")) {
            return Some("fixture exposed under another name");
        }
        // Without pytest installed, ty cannot bind a parameter to its fixture;
        // a same-named parameter somewhere is the next best evidence.
        if context.requested_as_parameter == NameUsage::Used
            && symbol.has_decorator(|d| d.name.last_segment() == "fixture")
        {
            return Some("fixture requested by a test parameter");
        }
        if PLUGIN_FIXTURES.contains(&name)
            && symbol.has_decorator(|d| d.name.last_segment() == "fixture")
        {
            return Some("overrides a fixture a pytest plugin provides");
        }
        if decorated_with_from(symbol, &["hookimpl"], true, PACKAGES) {
            return Some("pytest hook implementation");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Pytest;
    use crate::plugins::testing::Case;

    #[test]
    fn honours_configured_collection_patterns() {
        let configured = |name: &'static str, path: &'static str| {
            Case::function(name)
                .at(path)
                .collecting(&["check_*.py"], &["Check"], &["check_"])
        };
        assert!(configured("check_login", "/p/tests/check_auth.py").is_kept_by(&Pytest));
        assert!(
            !Case::function("check_login")
                .at("/p/tests/check_auth.py")
                .is_kept_by(&Pytest)
        );
        assert!(!configured("test_login", "/p/tests/test_auth.py").is_kept_by(&Pytest));
        assert!(
            Case::class("CheckSuite")
                .at("/p/tests/check_auth.py")
                .collecting(&["check_*.py"], &["Check"], &["check_"])
                .is_kept_by(&Pytest)
        );
    }

    #[test]
    fn keeps_fixtures_that_some_parameter_requests() {
        assert!(
            Case::function("client")
                .decorated("pytest.fixture")
                .requested_as_parameter()
                .is_kept_by(&Pytest)
        );
        assert!(
            !Case::function("client")
                .decorated("pytest.fixture")
                .is_kept_by(&Pytest)
        );
        assert!(
            !Case::function("client")
                .requested_as_parameter()
                .is_kept_by(&Pytest)
        );
    }

    #[test]
    fn keeps_test_methods_of_test_case_subclasses_anywhere() {
        assert!(
            Case::method("test_total")
                .at("/p/shop/checks.py")
                .with_ancestors(&["unittest.case.TestCase"])
                .is_kept_by(&Pytest)
        );
        assert!(
            Case::method("setUp")
                .at("/p/shop/checks.py")
                .with_ancestors(&["django.test.testcases.TestCase"])
                .is_kept_by(&Pytest)
        );
        assert!(
            !Case::method("make_order")
                .at("/p/shop/checks.py")
                .with_ancestors(&["unittest.case.TestCase"])
                .is_kept_by(&Pytest)
        );
        assert!(
            !Case::method("test_total")
                .at("/p/shop/checks.py")
                .is_kept_by(&Pytest)
        );
    }

    #[test]
    fn tests_py_is_a_test_module() {
        assert!(
            Case::function("test_it")
                .at("/p/shop/tests.py")
                .is_kept_by(&Pytest)
        );
    }

    #[test]
    fn keeps_overrides_of_plugin_fixtures() {
        assert!(
            Case::function("event_loop_policy")
                .decorated("pytest.fixture")
                .is_kept_by(&Pytest)
        );
        assert!(!Case::function("event_loop_policy").is_kept_by(&Pytest));
    }

    #[test]
    fn keeps_test_functions_in_test_files() {
        assert!(
            Case::function("test_it")
                .at("/p/tests/test_it.py")
                .is_kept_by(&Pytest)
        );
        assert!(
            Case::function("test_it")
                .at("/p/pkg/it_test.py")
                .is_kept_by(&Pytest)
        );
        assert!(
            Case::class("TestThing")
                .at("/p/tests/test_it.py")
                .is_kept_by(&Pytest)
        );
    }

    #[test]
    fn keeps_test_methods_and_lifecycle_hooks() {
        assert!(
            Case::method("test_it")
                .at("/p/tests/test_it.py")
                .is_kept_by(&Pytest)
        );
        assert!(
            Case::method("setUp")
                .at("/p/tests/test_it.py")
                .is_kept_by(&Pytest)
        );
        assert!(
            Case::method("setup_method")
                .at("/p/tests/test_it.py")
                .is_kept_by(&Pytest)
        );
        assert!(
            !Case::method("build")
                .at("/p/tests/test_it.py")
                .is_kept_by(&Pytest)
        );
        assert!(
            !Case::method("setUp")
                .at("/p/pkg/base.py")
                .is_kept_by(&Pytest)
        );
    }

    #[test]
    fn helpers_in_test_files_are_not_kept() {
        assert!(
            !Case::function("build_user")
                .at("/p/tests/test_it.py")
                .is_kept_by(&Pytest)
        );
    }

    #[test]
    fn test_named_functions_outside_test_files_are_not_kept() {
        assert!(
            !Case::function("test_it")
                .at("/p/pkg/mod.py")
                .is_kept_by(&Pytest)
        );
    }

    #[test]
    fn everything_in_conftest_is_kept() {
        assert!(
            Case::function("db")
                .at("/p/tests/conftest.py")
                .is_kept_by(&Pytest)
        );
        assert!(
            Case::variable("pytest_plugins")
                .at("/p/conftest.py")
                .is_kept_by(&Pytest)
        );
    }

    #[test]
    fn only_autouse_fixtures_are_kept_by_decorator() {
        assert!(
            Case::function("setup")
                .decorated_with_keywords("pytest.fixture", &["autouse"])
                .is_kept_by(&Pytest)
        );
        assert!(
            !Case::function("user")
                .decorated("pytest.fixture")
                .is_kept_by(&Pytest)
        );
    }

    #[test]
    fn fixtures_with_an_explicit_name_are_kept() {
        assert!(
            Case::function("get_mod")
                .decorated_with_keywords("pytest.fixture", &["name", "params"])
                .is_kept_by(&Pytest)
        );
    }

    #[test]
    fn hooks_are_kept() {
        assert!(Case::function("pytest_configure").is_kept_by(&Pytest));
        assert!(
            Case::function("run")
                .decorated("pytest.hookimpl")
                .is_kept_by(&Pytest)
        );
    }
}
