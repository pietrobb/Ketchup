"""Offline Session and host-only LiveSession clients; no production GUI attach.

Set PYTHONPATH to sdk/python, then ``from ketchup import Session``. The old
ketchup_sdk package is the separate plugin SDK, not this process client.
"""
from .client import (
    Document, HeadlessError, ProtocolError, Session, SessionClosedError,
    TransportError, TransportTimeout, rectangle,
)
from .live import LiveSession
__all__ = [
    "Document", "HeadlessError", "ProtocolError", "Session", "SessionClosedError",
    "TransportError", "TransportTimeout", "rectangle", "LiveSession",
]
