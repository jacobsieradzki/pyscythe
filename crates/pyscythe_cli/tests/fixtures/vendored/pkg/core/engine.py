from pkg._vendor.tinylib.api import connect


def start() -> str:
    return connect()


def stop() -> str:
    """Nothing calls this, and it is our code."""
    return "stopped"
