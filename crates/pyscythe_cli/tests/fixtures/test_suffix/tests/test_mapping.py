class Fixtures:
    """A base a project's own pytest plugin collects subclasses of."""


class MappedColumnTest(Fixtures):
    def test_maps(self):
        assert True


class HelperTests:
    def test_helps(self):
        assert True


class Helper:
    """Named for nothing in particular, so nothing collects it."""

    def build(self):
        return None
