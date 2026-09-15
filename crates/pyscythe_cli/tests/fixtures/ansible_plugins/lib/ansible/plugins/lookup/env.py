"""Found by name by the plugin loader, never imported."""

DOCUMENTATION = """
name: env
"""


class LookupModule:
    def run(self, terms, variables, **kwargs):
        return terms
