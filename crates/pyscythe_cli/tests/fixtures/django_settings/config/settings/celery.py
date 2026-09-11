import os

CELERY_BROKER_URL = os.environ.get("BROKER", "memory://")
if CELERY_BROKER_URL.startswith("memory"):
    CELERY_TASK_ALWAYS_EAGER = True
