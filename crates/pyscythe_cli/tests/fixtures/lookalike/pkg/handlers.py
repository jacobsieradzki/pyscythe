from pkg.local_router import router


@router.get("/nothing-registers-this")
def handler() -> None:
    pass
