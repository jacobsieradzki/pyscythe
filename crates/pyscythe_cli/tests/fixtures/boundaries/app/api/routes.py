from app.services.orders import place


class Request:
    pass


def handle() -> None:
    place(Request())
