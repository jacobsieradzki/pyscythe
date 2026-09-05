from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from pkg.d import D


def make() -> D:
    from pkg.d import D

    return D()
