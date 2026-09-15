"""Read by mypy, never run: the assertions are the return types."""

from pkg.core import build


def accepts_a_string() -> None:
    value: str = build()
    print(value)
