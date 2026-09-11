from __future__ import annotations

from typing import TYPE_CHECKING

from app.util import helper

if TYPE_CHECKING:
    from app.api.routes import Request


class Order:
    def total(self, request: Request) -> int:
        return helper()
