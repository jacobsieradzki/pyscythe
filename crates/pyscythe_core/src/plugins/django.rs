//! Django reaches code by file layout and by name: migrations, management
//! commands, app configs, models, settings, URL confs, and signal receivers.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::plugins::decorated_with;
use crate::symbol::SymbolKind;

pub(crate) struct Django;

/// `models.Model`, `AbstractUser`, `TimeStampedModel`, and similar, but not
/// the Pydantic and `SQLModel` bases that also end in "Model".
fn looks_like_django_model(base: &str) -> bool {
    !matches!(base, "BaseModel" | "SQLModel")
        && (base.ends_with("Model") || base.starts_with("Abstract"))
}

impl KeepRule for Django {
    fn plugin(&self) -> PluginName {
        PluginName::Django
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        let symbol = context.symbol;
        let name = symbol.name.as_str();
        let file_name = context.file_name();

        if context.parent_directory_is("migrations")
            && symbol.kind == SymbolKind::Class
            && name == "Migration"
        {
            return Some("Django migration");
        }
        if context.parent_directory_is("commands")
            && context.is_under_directory("management")
            && symbol.kind == SymbolKind::Class
            && name == "Command"
        {
            return Some("Django management command");
        }
        if file_name == "apps.py" && symbol.kind == SymbolKind::Class && name.ends_with("Config") {
            return Some("Django app config");
        }
        if (file_name == "models.py" || context.parent_directory_is("models"))
            && symbol.kind == SymbolKind::Class
            && symbol
                .bases
                .iter()
                .any(|base| looks_like_django_model(base.last_segment()))
        {
            return Some("Django model registered by the app registry");
        }
        if file_name.starts_with("settings")
            && matches!(symbol.kind, SymbolKind::Constant | SymbolKind::Variable)
        {
            return Some("Django setting read by name");
        }
        if file_name == "urls.py" && name == "urlpatterns" {
            return Some("Django URL configuration");
        }
        if (file_name == "wsgi.py" || file_name == "asgi.py") && name == "application" {
            return Some("Django application entry point");
        }
        if decorated_with(symbol, &["register"], true) {
            return Some("registered with the Django admin");
        }
        if decorated_with(symbol, &["receiver"], false) {
            return Some("connected as a Django signal receiver");
        }
        if (file_name == "tests.py" || context.is_under_directory("tests"))
            && symbol.kind == SymbolKind::Class
            && (name.starts_with("Test") || name.ends_with("Test") || name.ends_with("Tests"))
        {
            return Some("Django test case");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Django;
    use crate::plugins::testing::Case;

    #[test]
    fn keeps_layout_conventions() {
        assert!(
            Case::class("Migration")
                .at("/p/app/migrations/0001_initial.py")
                .is_kept_by(&Django)
        );
        assert!(
            Case::class("Command")
                .at("/p/app/management/commands/sync.py")
                .is_kept_by(&Django)
        );
        assert!(
            Case::class("ShopConfig")
                .at("/p/shop/apps.py")
                .is_kept_by(&Django)
        );
        assert!(
            Case::class("Order")
                .at("/p/shop/models.py")
                .extending("models.Model")
                .is_kept_by(&Django)
        );
        assert!(
            Case::class("User")
                .at("/p/shop/models/user.py")
                .extending("AbstractUser")
                .is_kept_by(&Django)
        );
        assert!(
            !Case::class("OrderSchema")
                .at("/p/shop/models.py")
                .extending("BaseModel")
                .is_kept_by(&Django)
        );
        assert!(
            !Case::class("Helper")
                .at("/p/shop/models.py")
                .is_kept_by(&Django)
        );
        assert!(
            Case::variable("DEBUG")
                .at("/p/site/settings.py")
                .is_kept_by(&Django)
        );
        assert!(
            Case::variable("urlpatterns")
                .at("/p/shop/urls.py")
                .is_kept_by(&Django)
        );
        assert!(
            Case::variable("application")
                .at("/p/site/wsgi.py")
                .is_kept_by(&Django)
        );
    }

    #[test]
    fn keeps_decorated_admin_and_receivers() {
        assert!(
            Case::class("OrderAdmin")
                .decorated("admin.register")
                .is_kept_by(&Django)
        );
        assert!(
            Case::function("on_save")
                .decorated("receiver")
                .is_kept_by(&Django)
        );
    }

    #[test]
    fn keeps_test_cases_but_not_helpers() {
        assert!(
            Case::class("OrderTests")
                .at("/p/shop/tests.py")
                .is_kept_by(&Django)
        );
        assert!(
            !Case::function("make_order")
                .at("/p/shop/tests.py")
                .is_kept_by(&Django)
        );
    }

    #[test]
    fn ordinary_classes_elsewhere_are_not_kept() {
        assert!(
            !Case::class("Migration")
                .at("/p/shop/views.py")
                .is_kept_by(&Django)
        );
        assert!(
            !Case::class("Order")
                .at("/p/shop/services.py")
                .is_kept_by(&Django)
        );
    }
}
