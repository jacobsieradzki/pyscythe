from sqlalchemy.orm import DeclarativeBase
from sqlmodel import SQLModel


class Base(DeclarativeBase):
    pass


class User(Base):
    __tablename__ = "users"


class Event(SQLModel, table=True):
    name: str


class EventRead(SQLModel):
    """A plain schema, not a table. Nothing uses it."""

    name: str
