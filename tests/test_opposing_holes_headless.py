"""Opposite-bore reports checked against native geometry, not validator internals."""
import math
import os
from pathlib import Path
import sys

import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "sdk" / "python"))
from ketchup import Session


@pytest.fixture
def session():
    configured = os.environ.get("KETCHUP_HEADLESS")
    if not configured:
        pytest.skip("set KETCHUP_HEADLESS to the built CLI")
    executable = Path(configured).resolve()
    worker = Path(os.environ.get("KETCHUP_EXACT_WORKER", str(executable.with_name(
        "ketchup-exact-worker.exe" if os.name == "nt" else "ketchup-exact-worker"))))
    assert executable.is_file() and worker.is_file()
    with Session(executable, worker, timeout=300) as session:
        yield session


def opposing(report):
    return [issue for issue in report["issues"]
            if issue["kind"].startswith("opposing_holes")]


def test_wall_report_agrees_with_native_volume_and_save_reopen(session, tmp_path):
    source = """depth=param('depth',10)
p=box('divider',(100,100,18))
hole(p,'z-',at=(40,50),diameter=8,depth=8,id='left')
hole(p,'z+',at=(40,50),diameter=8,depth=depth,id='right')
"""
    for depth, severity, removed_depth in [(11, "error", 18), (8, "warning", 16), (7, None, 15)]:
        document, report = session.program_document(source, params={"depth": depth})
        found = opposing(report)
        assert len(found) == (0 if severity is None else 1), report
        if severity:
            assert found[0]["severity"] == severity
            assert found[0]["parts"] == ["divider"]
            assert found[0]["where_mm"] is not None
        expected = 100 * 100 * 18 - math.pi * 4 ** 2 * removed_depth
        for reopened in [False, True]:
            if reopened:
                path = tmp_path / f"depth-{depth}.ketchup"
                document.save(path)
                document = session.open_document(path)
            result = document.evaluate(timeout_ms=300000)
            assert result["complete"] is True, result
            assert len(result["geometry"]) == 1
            assert result["geometry"][0]["native_evidence"]["volume_mm3"] == pytest.approx(expected, abs=1e-5)


def test_dowel_holes_from_both_sides_of_partition_need_staggering(session):
    source = """height=param('height',40)
p=board('divider',(18,100,100))
a=board('left',(100,100,18),at=(-100,0,40))
b=board('right',(100,100,18),at=(18,0,height))
dowels(a,p,dowel='8x30',count=2,margin=20)
dowels(p,b,dowel='8x30',count=2,margin=20)
"""
    report = session.check_program(source)
    found = opposing(report)
    assert len(found) == 2, report
    assert all(issue["parts"] == ["divider"] and issue["severity"] == "error" for issue in found)
    assert not opposing(session.check_program(source, params={"height": 60}))
