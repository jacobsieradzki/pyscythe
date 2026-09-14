"""Schema types whose hooks the GraphQL executor calls by name."""

import graphene


class Address(graphene.ObjectType):
    def resolve_country(self, info):
        return "GB"

    def __resolve_reference(self, info):
        return self

    def helper(self):
        """Nothing calls this."""
        return None


class CreateAddress(graphene.Mutation):
    def mutate(self, info, **data):
        return CreateAddress()


class Plain:
    def resolve_country(self, info):
        """Not a schema type, so nothing calls this either."""
        return None
