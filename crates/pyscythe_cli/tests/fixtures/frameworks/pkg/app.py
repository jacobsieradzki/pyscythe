from fastapi import FastAPI

from pkg.routes import router

app = FastAPI()
app.include_router(router)


@app.on_event("startup")
async def warm_up() -> None:
    return None
