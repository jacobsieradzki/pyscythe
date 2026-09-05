from fastapi import APIRouter

router = APIRouter()


@router.get("/users")
def list_users() -> list[str]:
    return []


@router.post("/users")
async def create_user() -> None:
    return None


def orphan() -> None:
    """Nothing calls this."""
