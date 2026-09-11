from app.api.routes import Request
from app.domain.order import Order


def place(request: Request) -> Order:
    return Order()
