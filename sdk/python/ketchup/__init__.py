"""Offline headless Session client.

Set PYTHONPATH to sdk/python, then ``from ketchup import Session``. The old
ketchup_sdk package is the separate plugin SDK, not this process client.
AI agents drive an open Kečup window through ``ketchup-app --mcp`` instead.
"""
from .client import (
    Document, HeadlessError, ProtocolError, Session, SessionClosedError,
    TransportError, TransportTimeout, rectangle,
)
__all__ = [
    "Document", "HeadlessError", "ProtocolError", "Session", "SessionClosedError",
    "TransportError", "TransportTimeout", "rectangle",
]
