from lib.core import Client

__all__ = ["Client", "connect"]


def connect() -> Client:
    return Client()


for __name in __all__:
    pass
