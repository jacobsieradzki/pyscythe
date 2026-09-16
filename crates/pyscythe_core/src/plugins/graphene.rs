//! Graphene and graphql-core call a schema type's hooks by name: the executor
//! looks up `resolve_<field>` for every field it resolves, `mutate` on a
//! mutation, and `__resolve_reference` when a federated gateway asks for an
//! entity. Nothing in the project calls them.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::symbol::SymbolKind;

pub(crate) struct Graphene;

/// Hooks the executor looks up on a schema type.
const SCHEMA_HOOKS: &[&str] = &[
    "mutate",
    "mutate_and_get_payload",
    "perform_mutation",
    "resolve_type",
    "resolve_id",
    "get_node",
    "get_queryset",
    "is_type_of",
    "subscribe",
];

/// The packages whose classes make a class a schema type.
const PACKAGES: &[&str] = &["graphene", "graphql", "graphene_django", "strawberry"];

/// Base names that mean a schema type when ty could not resolve the ancestry,
/// which is what happens when the project's dependencies are not installed.
fn looks_like_schema_base(base: &str) -> bool {
    matches!(
        base,
        "ObjectType" | "Mutation" | "InputObjectType" | "Interface" | "Subscription" | "Enum"
    ) || base.ends_with("ObjectType")
        || base.ends_with("Mutation")
}

/// Whether the class enclosing the symbol is part of a GraphQL schema.
fn is_schema_type(context: &KeepContext<'_>) -> bool {
    if PACKAGES
        .iter()
        .any(|package| context.ancestry.has_ancestor_in(package))
    {
        return true;
    }
    !context.ancestry.is_complete()
        && context
            .ancestry
            .names()
            .iter()
            .any(|base| looks_like_schema_base(base.last_segment()))
}

impl KeepRule for Graphene {
    fn plugin(&self) -> PluginName {
        PluginName::Graphene
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        let symbol = context.symbol;
        if crate::dead_code::is_attribute(symbol) && is_schema_type(&context) {
            return Some("a field of a GraphQL schema type, declared for the executor");
        }
        if symbol.kind != SymbolKind::Method {
            return None;
        }
        let name = symbol.name.as_str();
        let is_hook = name.starts_with("resolve_")
            || name.ends_with("__resolve_reference")
            || name.ends_with("__resolve_references")
            || SCHEMA_HOOKS.contains(&name);
        if is_hook && is_schema_type(&context) {
            return Some("GraphQL hook called by the schema executor");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Graphene;
    use crate::plugins::testing::Case;

    #[test]
    fn keeps_the_fields_a_schema_type_declares() {
        assert!(
            Case::attribute("staff_users")
                .on_class(
                    Case::class("AccountQueries")
                        .with_ancestors(&["graphene.types.objecttype.ObjectType"])
                )
                .is_kept_by(&Graphene)
        );
        assert!(
            !Case::attribute("staff_users")
                .on_class(Case::class("AccountQueries"))
                .is_kept_by(&Graphene)
        );
    }

    #[test]
    fn keeps_hooks_on_schema_types() {
        assert!(
            Case::method("resolve_country")
                .with_ancestors(&["graphene.types.objecttype.ObjectType"])
                .is_kept_by(&Graphene)
        );
        assert!(
            Case::method("mutate")
                .with_ancestors(&["graphene.types.mutation.Mutation"])
                .is_kept_by(&Graphene)
        );
    }

    #[test]
    fn keeps_hooks_when_the_base_is_only_a_name() {
        assert!(
            Case::method("resolve_country")
                .with_unresolved_ancestors(&["ModelObjectType"])
                .is_kept_by(&Graphene)
        );
    }

    #[test]
    fn leaves_ordinary_methods_and_other_classes_alone() {
        assert!(
            !Case::method("helper")
                .with_ancestors(&["graphene.types.objecttype.ObjectType"])
                .is_kept_by(&Graphene),
            "only the executor's hooks are kept"
        );
        assert!(
            !Case::method("resolve_country")
                .with_ancestors(&["shop.models.Address"])
                .is_kept_by(&Graphene),
            "a resolver-shaped method on an ordinary class is not a hook"
        );
    }
}
