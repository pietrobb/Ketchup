"""Offline Session plus discovery-capable, non-owning live GUI clients.

Set PYTHONPATH to sdk/python, then ``from ketchup import Session``. The old
ketchup_sdk package is the separate plugin SDK, not this process client.
"""
from .client import (
    Document, HeadlessError, ProtocolError, Session, SessionClosedError,
    TransportError, TransportTimeout, rectangle,
)
from .live import LiveConsentError, LiveSession, attach_live_instance, list_live_instances
__all__ = [
    "Document", "HeadlessError", "ProtocolError", "Session", "SessionClosedError",
    "TransportError", "TransportTimeout", "rectangle", "LiveConsentError",
    "LiveSession", "attach_live_instance", "list_live_instances",
]
