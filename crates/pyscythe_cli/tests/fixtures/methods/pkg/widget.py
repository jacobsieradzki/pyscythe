from typing import overload


class Widget:
    def __init__(self, size: int) -> None:
        self._size = size

    def render(self) -> str:
        return self._prepare()

    def _prepare(self) -> str:
        return "x" * self.size

    def unused_public(self) -> None:
        pass

    def _unused_private(self) -> None:
        pass

    def duck_typed(self) -> int:
        return 1

    def reflected(self) -> int:
        return 2

    @property
    def size(self) -> int:
        return self._size

    @size.setter
    def size(self, value: int) -> None:
        self._size = value

    @property
    def unused_property(self) -> int:
        return 0

    @overload
    def describe(self, value: int) -> str: ...
    @overload
    def describe(self, value: str) -> str: ...
    def describe(self, value: int | str) -> str:
        return str(value)


class Base:
    def hook(self) -> None:
        pass


class Child(Base):
    def hook(self) -> None:
        pass


class Registry(dict[str, int]):
    def keys(self):  # type: ignore[override]
        return super().keys()
