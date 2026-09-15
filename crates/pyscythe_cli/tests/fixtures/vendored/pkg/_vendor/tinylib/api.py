"""Carried verbatim from upstream; its unused parts are upstream's business."""

DEFAULT_TIMEOUT = 30


def connect():
    return "connected"


def disconnect():
    """Upstream exports this; this project never calls it."""
    return "disconnected"
