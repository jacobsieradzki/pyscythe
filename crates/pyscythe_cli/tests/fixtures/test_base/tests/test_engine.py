from proj.core import run
from proj.testing.fixtures import TestBase


class RegexpMySql(TestBase):
    """Collected by the project's plugin, though the name has no Test prefix."""

    def test_runs(self):
        assert run() == "ran"


class Detached:
    """Descends from nothing test-related, and nothing uses it."""

    def helper(self):
        return None
