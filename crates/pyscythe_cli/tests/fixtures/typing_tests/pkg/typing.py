"""A source module that happens to be called typing; its code still runs."""


def widen(value: str) -> str:
    return value


def forgotten() -> str:
    """Nothing calls this, and nothing type-checks it either."""
    return "dead"
