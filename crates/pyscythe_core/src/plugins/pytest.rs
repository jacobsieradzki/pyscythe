//! pytest collects tests by name, loads `conftest.py` implicitly, and runs
//! `autouse` fixtures without anyone naming them.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::plugins::decorated_with;
use crate::symbol::SymbolKind;

pub(crate) struct Pytest;

fn is_test_file(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".py") else {
        return false;
    };
    stem.starts_with("test_") || stem.ends_with("_test")
}

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
        if is_test_file(file_name) {
            if symbol.kind == SymbolKind::Function && name.starts_with("test") {
                return Some("collected as a test");
            }
            if symbol.kind == SymbolKind::Class && name.starts_with("Test") {
                return Some("collected as a test class");
            }
        }
        if symbol.kind == SymbolKind::Function && name.starts_with("pytest_") {
            return Some("pytest hook");
        }
        if symbol.has_decorator(|d| d.name.last_segment() == "fixture" && d.has_keyword("autouse"))
        {
            return Some("autouse fixture");
        }
        if decorated_with(symbol, &["hookimpl"], true) {
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
    fn hooks_are_kept() {
        assert!(Case::function("pytest_configure").is_kept_by(&Pytest));
        assert!(
            Case::function("run")
                .decorated("pytest.hookimpl")
                .is_kept_by(&Pytest)
        );
    }
}
