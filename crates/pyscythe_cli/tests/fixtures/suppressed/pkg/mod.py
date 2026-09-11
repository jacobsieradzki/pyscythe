def silenced_inline() -> None:  # pyscythe: ignore
    pass


# pyscythe: ignore[unused-function]
def silenced_above() -> None:
    pass


# pyscythe: ignore[unused-class]
def wrong_rule() -> None:
    pass


def loud() -> None:
    pass


def used() -> None:  # pyscythe: ignore
    pass
