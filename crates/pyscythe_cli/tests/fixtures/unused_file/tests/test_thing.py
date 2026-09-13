from tests.helpers import make_thing

APP = "apps/demo_app.py"


def test_thing() -> None:
    assert make_thing() == 1 and APP
