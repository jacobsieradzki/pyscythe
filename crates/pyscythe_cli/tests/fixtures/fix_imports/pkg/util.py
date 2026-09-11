import json
import os
from typing import Any, cast


def used() -> str:
    return json.dumps(cast(Any, {}))


def dead() -> str:
    return os.getcwd()
