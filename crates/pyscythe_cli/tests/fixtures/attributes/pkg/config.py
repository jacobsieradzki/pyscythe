from dataclasses import dataclass
from enum import Enum


class Settings:
    timeout = 30
    retries = 3
    _cache_size: int = 128
    # Declares a type and creates nothing, so there is nothing to call unused.
    declared_only: int

    def __init__(self) -> None:
        self.host = "localhost"
        self.port = 8080
        self._pool: list[str] = []

    def describe(self) -> str:
        return f"{self.host}:{self.timeout}"


@dataclass
class Point:
    x: int
    y: int


class Colour(Enum):
    RED = 1
    GREEN = 2
