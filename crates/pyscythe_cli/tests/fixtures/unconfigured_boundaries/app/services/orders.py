from app.api import routes
from app.domain.order import Order


def place(order: Order) -> str:
    return routes.NAME
