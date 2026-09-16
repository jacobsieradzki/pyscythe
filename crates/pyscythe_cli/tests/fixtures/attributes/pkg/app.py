from pkg.config import Colour, Point, Settings


def run() -> str:
    settings = Settings()
    point = Point(1, 2)
    return f"{settings.describe()}{settings.port}{point.x}{Colour.RED}"
