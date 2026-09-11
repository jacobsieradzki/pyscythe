class Router:
    def get(self, path: str):
        def wrap(fn):
            return fn

        return wrap


router = Router()
