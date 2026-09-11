import pytest

from lib.core import Client


@pytest.fixture(name="client")
def make_client() -> Client:
    return Client()


@pytest.fixture
def unnamed() -> int:
    return 1


def test_client(client: Client, unnamed: int) -> None:
    assert client is not None and unnamed
