"""Public, GUI-free Ketchup client. Each apply is atomic; a Python script is not.

Set PYTHONPATH to sdk/python, then ``from ketchup import Session``. The old
ketchup_sdk package is the separate plugin SDK, not this process client.
"""
from .client import (
    Document, HeadlessError, ProtocolError, Session, SessionClosedError,
    TransportError, TransportTimeout, rectangle,
)

__all__ = [
    "Document", "HeadlessError", "ProtocolError", "Session", "SessionClosedError",
    "TransportError", "TransportTimeout", "rectangle",
]
