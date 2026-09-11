from tests.helpers import make_thing


def test_thing() -> None:
    assert make_thing() == 1
