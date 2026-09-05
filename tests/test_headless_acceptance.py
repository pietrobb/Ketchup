"""S4 acceptance through public Python and real native CLI/worker processes.

Set KETCHUP_HEADLESS to the built CLI; KETCHUP_EXACT_WORKER defaults to its
sibling. Explicitly selected missing binaries FAIL, never skip. No build here.
The current CLI exposes native counts/volume, NOT serializable mesh triangles:
this is the weaker hole oracle, not an independent manifold/genus proof.
Exact CAD geometry and tolerant OBB support are NOT engineering certification.
"""
import json
import math
import os
from pathlib import Path
import sys
from unittest.mock import patch
import warnings

import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "sdk" / "python"))
from ketchup import HeadlessError, ProtocolError, Session
from ketchup.client import rectangle

# Declared analytic comparison tolerances, in millimetres and cubic millimetres.
VOLUME_ABS_MM3 = 1e-3
VOLUME_REL = 1e-9
BOUNDS_ABS_MM = 1e-6
HOLE_ORACLE_LIMITATION = (
    "Weaker S4 hole oracle: public evaluate has no mesh vertices/triangles; "
    "checked native worker topology [16,24,10,1,1] and analytic volume, "
    "NOT independently welded manifold triangulation/Euler 0/genus 1. "
    "BRep V-E+F is not a valid genus oracle for annular faces."
)


@pytest.fixture
def native_paths():
    configured = os.environ.get("KETCHUP_HEADLESS")
    if not configured:
        pytest.skip("Real S4 acceptance NOT RUN: set KETCHUP_HEADLESS to the built CLI")
    executable = Path(configured).resolve()
    suffix = ".exe" if os.name == "nt" else ""
    worker = Path(os.environ.get(
        "KETCHUP_EXACT_WORKER", str(executable.with_name("ketchup-exact-worker" + suffix))
    )).resolve()
    assert executable.is_file(), f"Selected CLI is missing: {executable}"
    assert worker.is_file(), f"Selected exact worker is missing: {worker}"
    return executable, worker


def assert_volume(actual, expected):
    assert math.isfinite(actual)
    assert actual == pytest.approx(expected, rel=VOLUME_REL, abs=VOLUME_ABS_MM3)


def assert_bound(report, state):
    for key in ("document_id", "revision", "canonical_digest"):
        assert report[key] == state[key], f"Stale {key} in evidence"


def canonical(state):
    # Revision and Undo cursor are session metadata, not persisted CAD history.
    # Ordered feature_ids in definitions preserve editable feature history.
    return {key: state[key] for key in (
        "document_id", "canonical_digest", "definitions", "features",
        "occurrences", "grounded_occurrence_ids",
    )}


def created_feature(result, kind):
    ids = result["created"]["feature_ids"]
    selected = [f["id"] for f in result["state"]["features"]
                if f["id"] in ids and f["kind"] == kind]
    feature_id, = selected
    return feature_id


def assert_translation(state, occurrence_id, xyz):
    occurrence, = [o for o in state["occurrences"] if o["id"] == occurrence_id]
    # CLI capabilities explicitly declare row-major 4x4 local transforms.
    x, y, z = xyz
    assert occurrence["transform"] == [
        1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, y,
        0.0, 0.0, 1.0, z, 0.0, 0.0, 0.0, 1.0,
    ]


def evaluate_current(doc, expected):
    """Check every terminal body producer, not just an aggregate success bit."""
    before = doc.state
    report = doc.evaluate()
    assert_bound(report, before)
    assert doc.state == before, "Evaluation changed canonical state or Undo"
    assert report["complete"] is True, report
    assert report["topology_complete"] is True, report
    assert report["not_evaluated"] is None, report
    producers = {(p["definition_id"], p["feature_id"]): p for p in report["producers"]}
    assert len(producers) == len(report["producers"]) == len(expected)
    assert set(producers) == set(expected)
    for producer in producers.values():
        for channel in ("render", "topology"):
            assert producer[channel]["status"] in {"current", "evaluated"}, producer

    evidence = {}
    for channel in ("geometry", "topology_geometry"):
        bodies = {(b["definition_id"], b["feature_id"]): b for b in report[channel]}
        assert len(bodies) == len(report[channel]) == len(expected)
        assert set(bodies) == set(expected)
        evidence[channel] = bodies
        for key, body in bodies.items():
            volume, bounds, topology = expected[key]
            assert_volume(body["mesh_signed_volume_mm3"], volume)
            assert len(body["bounds_mm"]) == 2
            for actual, wanted in zip(body["bounds_mm"], bounds):
                assert actual == pytest.approx(wanted, rel=0, abs=BOUNDS_ABS_MM)
            assert body["vertex_count"] > 0 and body["triangle_count"] > 0
            for field in ("result_fingerprint", "canonical_input_digest", "exact_input_digest",
                          "backend", "evaluator", "tolerance"):
                assert body[field], (field, body)
            # Graph topology evidence is mandatory, not fingerprint equality.
            # Rectangle render packages may legitimately have no native fields.
            native = body["native_evidence"]
            if channel == "topology_geometry" or topology == [16, 24, 10, 1, 1]:
                assert native is not None, body
            if native is not None:
                assert native["topology_counts"] == topology
                assert_volume(native["volume_mm3"], volume)
    return report, evidence


def gravity_current(doc, expected_state, unsupported_id=None):
    before = doc.state
    report = doc.validators.run(["gravity_support"])
    assert_bound(report, before)
    assert doc.state == before, "Validator changed state or Undo"
    assert report["schema"] == "ketchup.assistant-validation-context.v1"
    assert report["requested"] == report["executed"] == ["gravity_support"]
    assert report["state"] == expected_state, report
    assert report["complete"] is True and report["issues_complete"] is True, report
    assert report["not_evaluated"] == [] and report["unavailable_occurrences"] == []
    assert report["visible_occurrence_count"] == report["checked_occurrence_count"] == 2
    gravity = report["gravity_support"]
    assert gravity["state"] == expected_state and gravity["complete"] is True
    assert gravity["issues_complete"] is True and gravity["checked_occurrence_count"] == 2
    assert gravity["gravity_axis"] == "-Z"
    assert "only explicitly grounded participants seed support propagation" in gravity["assumptions"]
    assert any("OBB-SAT" in text for text in gravity["assumptions"])
    # floor_z_mm is legacy report metadata, NOT a synthetic supporting solid.
    count = 0 if unsupported_id is None else 1
    assert gravity["unsupported_count"] == report["issue_count"] == count
    assert report["issues"] == gravity["issues"]
    if unsupported_id is None:
        assert gravity["issues"] == []
    else:
        issue, = gravity["issues"]
        assert issue["code"] == "gravity.unsupported"
        assert issue["occurrence_id"] == unsupported_id and issue["name"] == "Second"
        assert issue["evidence_class"] == "exact", issue  # Unary rectangle issue, NOT mixed contact.
    # Normalize only the top-level current binding; retain ALL assumptions and
    # findings, including tolerant-vs-exact evidence and checked coverage.
    return {key: value for key, value in report.items()
            if key not in {"document_id", "revision", "canonical_digest"}}


def test_from_empty_hole_support_and_fresh_process_roundtrip(native_paths, tmp_path):
    saved_path = tmp_path / "s4-from-empty.ketchup"
    assert not saved_path.exists()
    warnings.warn(HOLE_ORACLE_LIMITATION, UserWarning, stacklevel=1)
    with Session(*native_paths) as session:
        doc = session.new_document()
        empty = doc.state
        assert empty["definitions"] == empty["features"] == empty["occurrences"] == []
        assert empty["grounded_occurrence_ids"] == []
        assert empty["undo_steps"] == empty["redo_steps"] == 0
        assert "gravity_support" in {v["id"] for v in doc.validators.list()["validators"]}
        base = doc.box("Foundation", 400, 200, 20, translation_mm=[0, 0, 0])
        definition_id, = base["created"]["definition_ids"]
        foundation_id, = base["created"]["occurrence_ids"]
        target = created_feature(base, "Pad")
        sketch = doc.create_sketch(definition_id, "Internal 40x40",
                                   rectangle(40, 40, origin_mm=(100, 60)))
        profile = created_feature(sketch, "Sketch")
        pocket = doc.pocket(definition_id, "Through hole", target, profile, 20)
        hole = created_feature(pocket, "Pocket")
        second = doc.box("Second", 20, 20, 100, translation_mm=[0, 0, 50])
        second_definition, = second["created"]["definition_ids"]
        second_id, = second["created"]["occurrence_ids"]
        second_feature = created_feature(second, "Pad")
        doc.set_grounded([foundation_id])
        floating = doc.state
        assert floating["grounded_occurrence_ids"] == [foundation_id]
        assert_translation(floating, foundation_id, [0, 0, 0])
        assert_translation(floating, second_id, [0, 0, 50])
        assert len(floating["occurrences"]) == len(floating["definitions"]) == 2
        expected = {
            (definition_id, hole): (400 * 200 * 20 - 40 * 40 * 20,
                                    [[0, 0, 0], [400, 200, 20]], [16, 24, 10, 1, 1]),
            (second_definition, second_feature): (40_000,
                                                  [[0, 0, 0], [20, 20, 100]], [8, 12, 6, 1, 1]),
        }
        floating_eval, _ = evaluate_current(doc, expected)
        gravity_current(doc, "failed", second_id)
        # Gap is 50 - 20 = 30 mm; this is a delta, not an absolute placement.
        doc.move([second_id], [0, 0, -30])
        supported = doc.state
        assert supported["canonical_digest"] != floating["canonical_digest"]
        assert supported["revision"] != floating_eval["revision"]
        assert supported["grounded_occurrence_ids"] == [foundation_id]
        assert_translation(supported, second_id, [0, 0, 20])
        evaluate_current(doc, expected)
        gravity_current(doc, "passed")

        # Negative control: merely touching world z=0 never seeds support.
        doc.set_grounded([foundation_id], False)
        ungrounded = doc.state
        no_ground = doc.validators.run(["gravity_support"])
        assert_bound(no_ground, ungrounded)
        assert no_ground["state"] == no_ground["gravity_support"]["state"] == "not_evaluated"
        assert no_ground["complete"] is False and no_ground["not_evaluated"]
        doc.set_grounded([foundation_id], True)
        _, saved_geometry = evaluate_current(doc, expected)
        saved_gravity = gravity_current(doc, "passed")
        before_save = doc.state
        doc.save(saved_path)
        assert doc.state == before_save
        assert saved_path.is_file() and saved_path.stat().st_size > 0
        saved_canonical = canonical(before_save)
        # Save is no-clobber by default, even for this session's own path.
        saved_bytes = saved_path.read_bytes()
        with pytest.raises(HeadlessError):
            doc.save(saved_path)
        assert saved_path.read_bytes() == saved_bytes and doc.state == before_save
    # Closing the first Session is essential: no registry/process cache survives.
    with Session(*native_paths) as fresh:
        reopened = fresh.open_document(saved_path)
        loaded = reopened.state
        assert canonical(loaded) == saved_canonical
        assert loaded["undo_steps"] == loaded["redo_steps"] == 0
        assert_translation(loaded, foundation_id, [0, 0, 0])
        assert_translation(loaded, second_id, [0, 0, 20])
        _, loaded_geometry = evaluate_current(reopened, expected)
        assert loaded_geometry == saved_geometry, "Geometry/backend/topology/fingerprint changed"
        assert gravity_current(reopened, "passed") == saved_gravity


def move_operation(occurrence_id, delta):
    return {"operation": "transform",
            "selector": {"type": "occurrences", "occurrence_ids": [occurrence_id]},
            "translation_mm": list(delta)}


def test_atomic_apply_and_undo_redo(native_paths):
    with Session(*native_paths) as session:
        doc = session.new_document()
        empty = doc.state
        created = doc.box("Transaction probe", 20, 20, 100)["created"]
        occurrence_id, = created["occurrence_ids"]
        before = doc.state
        absent_id = max(o["id"] for o in before["occurrences"]) + 10_000
        with pytest.raises(HeadlessError):
            # Valid first operation then a schema-valid nonexistent target:
            # catches partial application, not merely JSON parse rejection.
            doc.apply([move_operation(occurrence_id, [0, 0, 12]),
                       move_operation(absent_id, [0, 0, 1])])
        assert doc.state == before, "Rejected batch mutated state or added Undo"
        doc.undo()
        assert doc.state["canonical_digest"] == empty["canonical_digest"]
        doc.redo()
        assert doc.state["canonical_digest"] == before["canonical_digest"]
        before = doc.state
        doc.apply([move_operation(occurrence_id, [3, 0, 0]),
                   move_operation(occurrence_id, [0, 4, 0])])
        after = doc.state
        assert after["canonical_digest"] != before["canonical_digest"]
        assert after["undo_steps"] == before["undo_steps"] + 1
        assert_translation(after, occurrence_id, [3, 4, 0])
        doc.undo()
        assert doc.state["canonical_digest"] == before["canonical_digest"]
        doc.redo()
        assert doc.state["canonical_digest"] == after["canonical_digest"]
        # Only this negative protocol probe bypasses automatic public guards.
        # Actual server must reject stale revision AND stale digest individually.
        current = doc.state
        for revision, digest in ((before["revision"], current["canonical_digest"]),
                                 (current["revision"], before["canonical_digest"])):
            with pytest.raises(HeadlessError) as rejected:
                session._request("apply", {
                    "expected_revision": revision, "expected_digest": digest,
                    "program": {"operations": [move_operation(occurrence_id, [1, 0, 0])]},
                    "selection": [],
                })
            assert rejected.value.code == "stale_state"
            assert doc.state == current


def test_real_process_rejects_replayed_response(native_paths):
    # Replay actual old stdout at the transport boundary. The child and both
    # requests are real; only delivery of an old response is injected. S3 units
    # own timeout/exit/reaping tests, not this integration acceptance.
    with Session(*native_paths) as session:
        responses = []
        receive = session._responses.get

        def capture(*args, **kwargs):
            response = receive(*args, **kwargs)
            responses.append(response)
            return response

        with patch.object(session._responses, "get", side_effect=capture):
            session.capabilities()
        old_response = responses[0]
        assert isinstance(old_response, bytes)

        def replay(*args, **kwargs):
            current = receive(*args, **kwargs)
            assert isinstance(current, bytes)
            assert json.loads(current)["id"] != json.loads(old_response)["id"]
            return old_response

        with patch.object(session._responses, "get", side_effect=replay):
            with pytest.raises(ProtocolError, match="response id mismatch"):
                session.capabilities()


def test_missing_worker_cannot_claim_exact_evaluation(native_paths, tmp_path):
    executable, _ = native_paths
    missing = tmp_path / ("vanished-worker.exe" if os.name == "nt" else "vanished-worker")
    # Let public path resolution succeed, then remove the marker BEFORE any
    # evaluation. The CLI accepts --worker without spawning until evaluate;
    # no stub worker, fake geometry, or unrelated executable is launched.
    missing.touch()
    with Session(executable, missing) as session:
        missing.unlink()
        doc = session.new_document()
        made = doc.box("Missing worker probe", 20, 20, 100)
        definition_id, = made["created"]["definition_ids"]
        feature_id = created_feature(made, "Pad")
        before = doc.state
        report = doc.evaluate()
        assert_bound(report, before)
        assert doc.state == before
        assert report["complete"] is False and report["topology_complete"] is False
        assert report["not_evaluated"], report
        producer, = report["producers"]
        assert (producer["definition_id"], producer["feature_id"]) == (definition_id, feature_id)
        for channel in ("render", "topology"):
            assert producer[channel]["status"] in {"failed", "not_evaluated"}
            assert producer[channel]["reason"]
        assert report["geometry"] == report["topology_geometry"] == []
        # Pure canonical validators may still use tolerant fallback geometry;
        # they are deliberately NOT treated as proof that OCCT ran.


def test_persisted_colors_public_api_atomicity_and_geometry_cache(native_paths, tmp_path):
    saved = tmp_path / "colors.ketchup"
    with Session(*native_paths) as session:
        doc = session.new_document()
        created = doc.box("Colored part", 10, 20, 30)
        occurrence_id, = created["created"]["occurrence_ids"]
        baseline = doc.state
        evaluated = doc.evaluate()
        assert evaluated["complete"] is True
        colored = doc.set_color([occurrence_id], [0, 128, 255])["state"]
        assert colored["occurrences"][0]["color"] == [0, 128, 255]
        assert colored["canonical_digest"] != baseline["canonical_digest"]
        cached = doc.evaluate()
        assert all(p[channel]["status"] == "current" for p in cached["producers"] for channel in ("render", "topology"))
        assert cached["geometry"] == evaluated["geometry"]
        assert doc.undo()["state"]["occurrences"][0]["color"] is None
        assert doc.redo()["state"]["occurrences"][0]["color"] == [0, 128, 255]
        for bad in ([256, 0, 0], [-1, 0, 0], [1.5, 0, 0], [1, 2], [True, 0, 0]):
            before = doc.state
            with pytest.raises(HeadlessError):
                doc.set_color([occurrence_id], bad)
            assert doc.state == before
        before = doc.state
        with pytest.raises(HeadlessError):
            doc.set_color([occurrence_id, 999999], None)
        assert doc.state == before
        with pytest.raises(HeadlessError) as stale:
            session._request("apply", {"expected_revision": baseline["revision"], "expected_digest": baseline["canonical_digest"],
                "program": {"operations": [{"operation": "set_color", "selector": {"type": "occurrences", "occurrence_ids": [occurrence_id]}, "color": None}]}})
        assert stale.value.code == "stale_state"
        doc.copy([occurrence_id], [50, 0, 0])
        assert all(o["color"] == [0, 128, 255] for o in doc.state["occurrences"])
        doc.set_color([occurrence_id], None)
        doc.save(saved)
        expected = canonical(doc.state)
    with Session(*native_paths) as session:
        assert canonical(session.open_document(saved).state) == expected
