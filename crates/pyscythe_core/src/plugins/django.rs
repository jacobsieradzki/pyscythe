//! Django reaches code by file layout and by name: migrations, management
//! commands, app configs, models, settings, URL confs, and signal receivers.

use crate::keep::{FileRole, KeepContext, KeepRule, PluginName};
use crate::plugins::decorated_with_from;
use crate::symbol::SymbolKind;

pub(crate) struct Django;

/// Methods Django, the admin, class-based views, and DRF call by name.
const HOOK_METHODS: &[&str] = &[
    // Models and forms
    "save",
    "delete",
    "clean",
    "clean_fields",
    "full_clean",
    "get_absolute_url",
    "natural_key",
    // The user model protocol, read by auth and the admin
    "is_staff",
    "is_active",
    "is_superuser",
    "is_anonymous",
    "is_authenticated",
    "has_perm",
    "has_perms",
    "has_module_perms",
    "get_full_name",
    "get_short_name",
    "get_username",
    "get_session_auth_hash",
    "set_password",
    "check_password",
    "set_unusable_password",
    "has_usable_password",
    "save_model",
    "save_formset",
    "save_related",
    // Admin and views
    "get_queryset",
    "get_object",
    "get_context_data",
    "get_form",
    "get_form_class",
    "get_form_kwargs",
    "get_success_url",
    "get_template_names",
    "get_urls",
    "get_readonly_fields",
    "get_list_display",
    "get_fieldsets",
    "get_search_results",
    "form_valid",
    "form_invalid",
    "dispatch",
    "setup",
    "get",
    "post",
    "put",
    "patch",
    "delete",
    "head",
    "options",
    "trace",
    // Django REST framework
    "list",
    "create",
    "retrieve",
    "update",
    "partial_update",
    "destroy",
    "perform_create",
    "perform_update",
    "perform_destroy",
    "get_serializer",
    "get_serializer_class",
    "get_serializer_context",
    "get_permissions",
    "get_authenticators",
    "get_throttles",
    "get_paginated_response",
    "paginate_queryset",
    "filter_queryset",
    "validate",
    "to_representation",
    "to_internal_value",
    "has_permission",
    "has_object_permission",
    // Management commands and middleware
    "handle",
    "add_arguments",
    "process_request",
    "process_response",
    "process_view",
    "process_exception",
    "process_template_response",
    // App config and signals
    "ready",
    // Custom user managers
    "create_user",
    "create_superuser",
    "get_by_natural_key",
];

/// `validate_<field>`, `clean_<field>`, `get_<field>_display`-style hooks.
fn is_prefixed_hook(name: &str) -> bool {
    name.starts_with("validate_")
        || name.starts_with("clean_")
        || name.starts_with("formfield_for_")
        || name.starts_with("has_") && name.ends_with("_permission")
}

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
        if symbol.kind == SymbolKind::Class && context.ancestry.has_ancestor_in("django.db.models")
        {
            return Some("Django model registered by the app registry");
        }
        if (file_name == "models.py" || context.parent_directory_is("models"))
            && symbol.kind == SymbolKind::Class
            && !context.ancestry.is_complete()
            && symbol
                .bases
                .iter()
                .any(|base| looks_like_django_model(base.last_segment()))
        {
            return Some("Django model registered by the app registry");
        }
        if (file_name.starts_with("settings") || context.file_role == FileRole::DjangoSettings)
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
        if symbol.kind == SymbolKind::Method
            && (HOOK_METHODS.contains(&name) || is_prefixed_hook(name))
        {
            return Some("Django hook method called by name");
        }
        if decorated_with_from(symbol, &["register"], true, &["django"]) {
            return Some("registered with the Django admin");
        }
        if decorated_with_from(symbol, &["receiver"], false, &["django"]) {
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
    fn keeps_settings_by_file_role_wherever_they_live() {
        assert!(
            Case::variable("DEBUG")
                .at("/p/config/django/base.py")
                .in_django_settings()
                .is_kept_by(&Django)
        );
        assert!(
            !Case::variable("DEBUG")
                .at("/p/config/django/base.py")
                .is_kept_by(&Django)
        );
        assert!(Case::method("create_superuser").is_kept_by(&Django));
    }

    #[test]
    fn keeps_hook_methods_but_not_ordinary_ones() {
        assert!(Case::method("save").is_kept_by(&Django));
        assert!(Case::method("get_queryset").is_kept_by(&Django));
        assert!(Case::method("validate_email").is_kept_by(&Django));
        assert!(Case::method("has_change_permission").is_kept_by(&Django));
        assert!(!Case::method("compute_total").is_kept_by(&Django));
        assert!(
            !Case::function("save").is_kept_by(&Django),
            "only methods are hooks"
        );
    }

    #[test]
    fn resolved_django_models_are_kept_anywhere_and_local_lookalikes_are_not() {
        let model = Case::class("Order")
            .at("/p/shop/domain.py")
            .with_ancestors(&["django.db.models.base.Model", "builtins.object"]);
        assert!(model.is_kept_by(&Django));

        let lookalike = Case::class("Order")
            .at("/p/shop/models.py")
            .extending("BaseModel")
            .with_ancestors(&["pkg.core.BaseModel", "builtins.object"]);
        assert!(!lookalike.is_kept_by(&Django));
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
