import sys

if sys.platform == "win32":
    from app import windows as backend
elif sys.platform == "darwin":
    from app import macos as backend
else:
    from app import linux as backend


def resolve() -> str:
    return backend.original_addr()
