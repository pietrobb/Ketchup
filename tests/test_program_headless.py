"""Rule programs through the Python SDK and the real headless CLI.

Set KETCHUP_HEADLESS to the built CLI; KETCHUP_EXACT_WORKER defaults to its
sibling.
"""
import math
import os
from pathlib import Path
import sys

import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "sdk" / "python"))
from ketchup import HeadlessError, Session

CABINET = (ROOT / "examples" / "programs" / "cabinet.star").read_text(encoding="utf-8")


@pytest.fixture
def session():
    configured = os.environ.get("KETCHUP_HEADLESS")
    if not configured:
        pytest.skip("set KETCHUP_HEADLESS to the built CLI")
    executable = Path(configured).resolve()
    suffix = ".exe" if os.name == "nt" else ""
    worker = Path(os.environ.get(
        "KETCHUP_EXACT_WORKER", str(executable.with_name("ketchup-exact-worker" + suffix))
    )).resolve()
    with Session(executable, worker, timeout=300) as session:
        yield session


def test_check_reports_issues_parts_and_hardware_without_a_document(session):
    report = session.check_program(CABINET, params={"width": 700})
    assert report["ok"] is True
    assert report["bom"]["total_parts"] == 7
    assert report["bom"]["hardware"] == [{"item": "dowel 8x30", "count": 8}]

    broken = session.check_program('a = box("a", (100, 100, 18))\nb = box("b", (100, 100, 18), at = (50, 0, 0))\n')
    assert broken["ok"] is False
    assert broken["issues"][0]["kind"] == "collision"
    assert broken["issues"][0]["parts"] == ["a", "b"]


def test_interpreter_errors_name_the_line(session):
    with pytest.raises(HeadlessError) as error:
        session.check_program('box("a", (1, 2, 3))\nbox("a", (1, 2, 3))\n', file_name="dup.star")
    assert error.value.code == "program.evaluation_error"
    assert "dup.star:2" in str(error.value)


def test_program_document_builds_exact_parts_as_one_undo_step(session, tmp_path):
    document, report = session.program_document(CABINET, file_name="cabinet.star")
    assert report["ok"] is True
    state = document.state
    assert state["undo_steps"] == 1
    assert len(state["occurrences"]) == 7

    evaluation = document.evaluate(timeout_ms=300000)
    assert evaluation["complete"] is True
    sides = [body for body in evaluation["geometry"]
             if body["bounds_mm"] == [[0.0, 0.0, 0.0], [18.0, 350.0, 720.0]]]
    assert len(sides) == 2
    # 18 x 350 x 720 minus four 8 mm dowel holes (16 deep), 32 shelf-pin holes
    # (5 mm, 10 deep) and a 4 x 8 mm groove over the full height.
    expected = (18 * 350 * 720
                - 4 * math.pi * 4 ** 2 * 16
                - 32 * math.pi * 2.5 ** 2 * 10
                - 4 * 8 * 720)
    for side in sides:
        assert side["native_evidence"]["volume_mm3"] == pytest.approx(expected, rel=1e-6)

    path = tmp_path / "cabinet.ketchup"
    document.save(path)
    reopened = session.open_document(path)
    assert len(reopened.state["occurrences"]) == 7
