from collections.abc import Callable
from http.server import BaseHTTPRequestHandler


class Registry:
    routes: dict[str, Callable[[], None]] = {}

    @classmethod
    def handler(cls, method: str) -> Callable[[Callable[[], None]], Callable[[], None]]:
        def register(func: Callable[[], None]) -> Callable[[], None]:
            cls.routes[method] = func
            return func

        return register


@Registry.handler("GET")
def get_headers() -> None:
    pass


class MockHandler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        pass

    def helper(self) -> None:
        pass


class Visitor:
    def visit(self, node: object) -> None:
        getattr(self, f"visit_{type(node).__name__}")(node)

    def visit_Name(self, node: object) -> None:
        pass
