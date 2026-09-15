from pkg.core import run


def test_runs():
    assert run() == "ran"


def forgotten_helper():
    """A dead helper in a real test module is still dead."""
    return None
