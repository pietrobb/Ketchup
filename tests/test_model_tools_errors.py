"""Malformed requests must not turn bounded CAD tools into context dumps."""
import json
import os
from pathlib import Path
import sys

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "sdk" / "python"))
from ketchup import HeadlessError, Session

EXECUTABLE = os.environ.get("KETCHUP_HEADLESS")
pytestmark = pytest.mark.skipif(not EXECUTABLE, reason="requires the newly built headless executable")


@pytest.mark.parametrize("method,params", [
    ("query", {"kind": "occurrences", "x" * 40000: True}),
    ("detail", {"kind": "x" * 40000, "id": 1}),
    ("query", {"kind": "occurrences", "\u0001" * 12000: True}),
])
def test_malformed_query_diagnostics_are_bounded_and_nondestructive(method, params):
    with Session(executable=EXECUTABLE, compact=True) as session:
        doc = session.new_document()
        before = doc.summary()
        with pytest.raises(HeadlessError) as caught:
            session._request(method, params)
        error = caught.value
        assert error.code == "invalid_params"
        assert error.details["diagnostic_truncated"] is True
        assert len(json.dumps({"code": error.code, "message": error.message,
                               "details": error.details}).encode("utf-8")) < 32768
        assert doc.summary() == before
