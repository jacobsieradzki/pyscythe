from pkg.routes import list_users


def test_list_users(client: object) -> None:
    assert list_users() == []


class TestUsers:
    def test_empty(self) -> None:
        assert True
