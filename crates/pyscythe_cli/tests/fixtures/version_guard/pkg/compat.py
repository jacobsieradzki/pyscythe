import sys

if sys.version_info >= (3, 99):
    import futuremodule
else:
    futuremodule = None

if sys.version_info < (3, 11):
    import legacy_backport
else:
    legacy_backport = None

PY_3_99_PLUS = sys.version_info[:2] >= (3, 99)

if PY_3_99_PLUS:
    import flagged_future_module
else:
    flagged_future_module = None

import missingmodule


def use():
    return (futuremodule, legacy_backport, flagged_future_module, missingmodule)
