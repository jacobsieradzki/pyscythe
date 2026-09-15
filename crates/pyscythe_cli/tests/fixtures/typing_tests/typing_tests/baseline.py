"""The same, in the directory attrs uses."""

from pkg.core import build


def returns_a_string() -> None:
    reveal_type(build())  # noqa: F821
