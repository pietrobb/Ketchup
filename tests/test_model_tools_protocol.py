"""Real-process compact API regression; no native geometry claim."""
import json
import os
from pathlib import Path
import sys
import time

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "sdk" / "python"))
from ketchup import Session, HeadlessError

EXECUTABLE = os.environ.get("KETCHUP_HEADLESS")
pytestmark = pytest.mark.skipif(not EXECUTABLE, reason="set KETCHUP_HEADLESS to the newly built executable")


def bounded(result, budget=32768):
    size = len(json.dumps(result, ensure_ascii=False).encode("utf-8"))
    assert size < budget
    return size


def test_real_compact_cycle_and_cursor_invalidation(tmp_path):
    with Session(executable=EXECUTABLE, compact=True) as session:
        requests = []
        original = session._request

        def trace(method, params=None, **kwargs):
            requests.append(method)
            return original(method, params, **kwargs)

        session._request = trace
        doc = session.new_document()
        assert requests == ["summary", "new"]
        receipt = doc.box("Test panel", 20, 30, 4)
        assert "occurrences" not in receipt["state"]
        occurrence = receipt["created"]["occurrence_ids"]["ids"][0]
        assert receipt["created"]["occurrence_ids"]["complete"]
        bounded(receipt)
        pattern = doc.apply([{"operation": "linear_pattern", "selector": {
            "type": "occurrences", "occurrence_ids": [occurrence]},
            "instances": 151, "step_mm": [30, 0, 0]}])
        bounded(pattern)
        assert pattern["created"]["occurrence_ids"]["complete"] is False
        assert pattern["created"]["occurrence_ids"]["total"] == 150
        before = doc.summary()
        bounded(before, 8192)
        first = doc.query(kind="occurrences", limit=30)
        cursor = first["next_cursor"]
        assert cursor and first["total_matches"] == 151
        assert len(first["items"]) == 30
        bounded(first)
        ids = [item["id"] for item in first["items"]]
        while cursor:
            page = doc.query(kind="occurrences", limit=30, cursor=cursor)
            bounded(page)
            ids.extend(item["id"] for item in page["items"])
            cursor = page["next_cursor"]
        assert len(ids) == len(set(ids)) == 151
        assert doc.summary() == before
        detail = doc.detail("occurrences", occurrence)
        assert detail["item"]["id"] == occurrence
        moved = doc.move([occurrence], [0, 0, 2])
        bounded(moved)
        for after_undo in [False, True]:
            if after_undo:
                doc.undo()
            with pytest.raises(HeadlessError) as error:
                doc.query(kind="occurrences", limit=30, cursor=first["next_cursor"])
            assert error.value.code == "stale_cursor"
        assert doc.summary()["state"]["canonical_digest"] == before["state"]["canonical_digest"]
        doc.redo()
        target = tmp_path / "compact-model.ketchup"
        bounded(doc.save(target))
        opened = session.open_document(target)
        assert opened.summary()["summary"]["counts"]["root_occurrences"] == 151
        with pytest.raises(HeadlessError) as error:
            opened.query(kind="occurrences", limit=30, cursor=first["next_cursor"])
        assert error.value.code == "stale_cursor"
        assert "state" not in requests


def test_real_ten_thousand_catalog_via_public_compact_api(tmp_path):
    start = time.perf_counter()
    sizes = []
    with Session(executable=EXECUTABLE, compact=True, timeout=120) as session:
        doc = session.new_document()
        seed = doc.box("Repeated component", 1, 1, 1)
        occurrence = seed["created"]["occurrence_ids"]["ids"][0]
        for count in [500] * 19 + [499]:
            sizes.append(bounded(doc.apply([{"operation": "linear_pattern", "selector": {
                "type": "occurrences", "occurrence_ids": [occurrence]},
                "instances": count + 1, "step_mm": [2, 0, 0]}])))
        summary = doc.summary()
        assert summary["summary"]["counts"]["root_occurrences"] == 10000
        summary_bytes = bounded(summary, 8192)
        build_seconds = time.perf_counter() - start
        start = time.perf_counter()
        seen = set()
        cursor = None
        pages = 0
        while True:
            page = doc.query(kind="occurrences", limit=100, cursor=cursor)
            sizes.append(bounded(page))
            page_ids = [item["id"] for item in page["items"]]
            assert not seen.intersection(page_ids)
            seen.update(page_ids)
            pages += 1
            cursor = page["next_cursor"]
            if cursor is None:
                break
        assert len(seen) == 10000
        assert doc.summary() == summary
        report = {"scope": "10000 repeated root instances, not unique solids or collision certification",
                  "count": len(seen), "pages": pages, "summary_bytes": summary_bytes,
                  "max_response_bytes": max(sizes), "build_seconds": build_seconds,
                  "traversal_seconds": time.perf_counter() - start}
        print(json.dumps(report))
        (tmp_path / "model-query-measurement.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
