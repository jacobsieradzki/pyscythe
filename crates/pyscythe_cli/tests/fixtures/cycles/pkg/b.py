from pkg.a import from_a


def from_b() -> int:
    return 1


def call_a() -> int:
    return from_a()
