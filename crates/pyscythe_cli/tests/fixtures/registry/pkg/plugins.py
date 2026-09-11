REGISTRY: dict[str, type] = {}


class Plugin:
    def __init_subclass__(cls, **kwargs) -> None:
        REGISTRY[cls.__name__] = cls


class Alpha(Plugin):
    pass


class Plain:
    pass
