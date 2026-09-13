"""Offline public-client tests. These do not claim OCCT/native geometry evidence."""
import io
import json
from pathlib import Path
import queue
import sys
import threading
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "sdk" / "python"))
from ketchup import Session, HeadlessError, ProtocolError, SessionClosedError, TransportTimeout
from ketchup.client import PROTOCOL, MAX_LINE_BYTES, rectangle


class Pipe(io.RawIOBase):
    def __init__(self):
        super().__init__()
        self.q = queue.Queue()
        self.pending = bytearray()

    def readable(self):
        return True

    def readinto(self, buffer):
        if not self.pending:
            item = self.q.get()
            if item is None:
                return 0
            self.pending.extend(item)
        n = min(len(buffer), len(self.pending))
        buffer[:n] = self.pending[:n]
        del self.pending[:n]
        return n

    def read(self, size=-1):
        buffer = bytearray(size if size >= 0 else 4096)
        n = self.readinto(buffer)
        return bytes(buffer[:n])


class Input:
    def __init__(self, process):
        self.process = process
    def write(self, data):
        request = json.loads(bytes(data))
        self.process.requests.append(request)
        response = self.process.answer(request)
        if response is not None:
            self.process.stdout.q.put(response)
        return len(data)
    def flush(self):
        pass
    def close(self):
        pass


class FakeProcess:
    def __init__(self, answer=None):
        self.stdout, self.stderr = Pipe(), Pipe()
        self.stdin = Input(self)
        self.requests = []
        self.stopped = False
        self.waited = False
        self.revision = 0
        self.answer = answer or self.normal
    def normal(self, request):
        method = request["method"]
        if method in {"new", "open", "apply", "undo", "redo", "set_grounded"}:
            self.revision += 1
        state = {"document_id": 1, "revision": self.revision, "canonical_digest": str(self.revision),
                 "definitions": [], "features": [], "occurrences": [], "grounded_occurrence_ids": [],
                 "undo_steps": 0, "redo_steps": 0}
        return self.result(request, {"state": state, "created": {"definition_ids": [11], "occurrence_ids": [22], "feature_ids": [33]}})
    @staticmethod
    def result(request, result):
        return (json.dumps({"protocol": PROTOCOL, "id": request["id"], "result": result}) + "\n").encode()
    def poll(self):
        return 0 if self.stopped else None
    def terminate(self):
        self.stopped = True
        self.stdout.q.put(None)
        self.stderr.q.put(None)
    kill = terminate
    def wait(self, timeout=None):
        self.waited = True
        return 0


class ClientTests(unittest.TestCase):
    def session(self, process, **kwargs):
        patcher = patch("ketchup.client.subprocess.Popen", return_value=process)
        popen = patcher.start()
        self.addCleanup(patcher.stop)
        session = Session(executable=sys.executable, worker=sys.executable, **kwargs)
        self.addCleanup(session.close)
        self.assertFalse(popen.call_args.kwargs["shell"])
        self.assertEqual(popen.call_args.args[0][1], "--stdio")
        return session

    def test_helpers_observed_guards_and_reaping(self):
        process = FakeProcess()
        with self.session(process) as session:
            doc = session.new_document()
            result = doc.box("base", 10, 20, 3)
            self.assertEqual(result["created"]["occurrence_ids"], [22])
            request = process.requests[-1]
            self.assertEqual(request["params"]["expected_revision"], 1)
            operation = request["params"]["program"]["operations"][0]
            self.assertEqual(operation["operation"], "create_part")
            self.assertEqual(operation["entities"], rectangle(10, 20))
            self.assertEqual(operation["feature"], {"type": "extrusion", "distance_mm": 3})
            doc.move([22], [0, 0, 10])
            self.assertEqual(process.requests[-1]["params"]["expected_digest"], "2")
        self.assertTrue(process.stopped and process.waited)
        with self.assertRaises(SessionClosedError):
            doc.undo()

    def test_surface_helpers_emit_only_typed_append_feature_operations(self):
        process = FakeProcess()
        doc = self.session(process).new_document()
        calls = [
            (lambda: doc.planar_surface(2, "Planar", 11), {
                "type": "surface_body",
                "source": {"type": "planar", "profile_feature_id": 11},
            }),
            (lambda: doc.loft_surface(
                2,
                "Loft",
                [{"profile_feature_id": 11, "elevation_mm": 0},
                 {"profile_feature_id": 12, "elevation_mm": 20}],
                continuity="tangent",
            ), {
                "type": "surface_body",
                "source": {
                    "type": "loft",
                    "sections": [{"profile_feature_id": 11, "elevation_mm": 0},
                                 {"profile_feature_id": 12, "elevation_mm": 20}],
                    "continuity": "tangent",
                },
            }),
            (lambda: doc.trim_surface(2, "Trim", 21, 22), {
                "type": "surface_trim", "target_feature_id": 21,
                "cutter_feature_id": 22,
            }),
            (lambda: doc.extend_surface(2, "Extend", 23, 2.5), {
                "type": "surface_extend", "target_feature_id": 23,
                "distance_mm": 2.5,
            }),
            (lambda: doc.knit_surfaces(
                2, "Knit", [24, 25], tolerance_mm=0.001, make_solid=True
            ), {
                "type": "surface_knit", "surface_feature_ids": [24, 25],
                "tolerance_mm": 0.001, "make_solid": True,
            }),
            (lambda: doc.thicken_surface(
                2, "Thicken", 26, 1.5, direction="symmetric"
            ), {
                "type": "surface_thicken", "target_feature_id": 26,
                "thickness_mm": 1.5, "direction": "symmetric",
            }),
        ]
        for call, expected_feature in calls:
            call()
            request = process.requests[-1]
            self.assertEqual(request["method"], "apply")
            operation, = request["params"]["program"]["operations"]
            self.assertEqual(operation["operation"], "append_feature")
            self.assertEqual(operation["definition_id"], 2)
            self.assertEqual(operation["feature"], expected_feature)

    def test_cam_setup_helper_emits_one_typed_guarded_assistant_operation(self):
        process = FakeProcess()
        doc = self.session(process).new_document()
        doc.cam_setup(
            7, "Reviewed top setup", 2, 33,
            stock_minimum_mm=(-2, -2, -1), stock_maximum_mm=(52, 32, 12),
            tool_number=1, tool_kind="flat_end_mill", tool_diameter_mm=6,
            flute_length_mm=18, overall_length_mm=50, holder_diameter_mm=20,
            holder_length_mm=35, spindle_rpm=12000, feed_mm_per_min=900,
            plunge_mm_per_min=250, safe_height_mm=8, maximum_stepdown_mm=2,
            stepover_ratio=0.45, radial_allowance_mm=0.2, axial_allowance_mm=0.1,
        )
        request = process.requests[-1]
        self.assertEqual(request["method"], "apply")
        self.assertEqual(request["params"]["expected_revision"], 1)
        operation, = request["params"]["program"]["operations"]
        self.assertEqual(operation["operation"], "upsert_cam_plan")
        self.assertEqual(operation["target_feature_id"], 33)
        self.assertEqual(operation["work_offset"], "g54")
        self.assertEqual(operation["x_axis"], [1, 0, 0])
        self.assertEqual(operation["spindle_rpm"], 12000)

    def test_cam_review_and_confirmed_no_clobber_export_are_guarded(self):
        process = FakeProcess()
        doc = self.session(process).new_document()
        operation = {
            "type": "face", "id": 1, "minimum_mm": [1, 1],
            "maximum_mm": [19, 9], "target_z_mm": 5,
        }
        doc.cam_preview(7, [operation], fixtures=[], dialect="controller_neutral_json")
        preview = process.requests[-1]
        self.assertEqual(preview["method"], "cam_preview")
        self.assertEqual(preview["params"]["expected_revision"], 1)
        self.assertEqual(preview["params"]["operations"], [operation])
        self.assertEqual(preview["params"]["dialect"], "controller_neutral_json")

        doc.cam_export("cam-review-token", "program.json", confirmed=True)
        export = process.requests[-1]
        self.assertEqual(export["method"], "cam_export")
        self.assertEqual(export["params"]["expected_digest"], "1")
        self.assertTrue(export["params"]["confirmed"])
        self.assertNotIn("overwrite", export["params"])

        with self.assertRaises(ValueError):
            doc.cam_preview(7, [], dialect="iso_metric_gcode")
        with self.assertRaises(ValueError):
            doc.cam_export("", "program.nc", confirmed=True)

    def test_fea_review_helper_emits_guarded_bounded_confirmed_study(self):
        process = FakeProcess()
        doc = self.session(process).new_document()
        levels = [
            {"surface_deflection_mm": 0.5, "angular_deflection_rad": 0.5,
             "max_tetrahedra": 128, "max_relative_volume_error": 0.05,
             "min_tetrahedron_quality": 1.0e-5},
            {"surface_deflection_mm": 0.2, "angular_deflection_rad": 0.2,
             "max_tetrahedra": 256, "max_relative_volume_error": 0.02,
             "min_tetrahedron_quality": 1.0e-5},
        ]
        doc.fea_review(
            2, 33, 7, "pressure-case",
            material={"id": 1, "youngs_modulus_mpa": 200000,
                      "poisson_ratio": 0.3, "yield_strength_mpa": 250},
            constrained_face_ordinals=[0],
            face_tractions=[{"face_ordinal": 1,
                             "traction_local_n_per_mm2": [0, 0, -1]}],
            mesh_levels=levels, confirmed=True,
        )
        request = process.requests[-1]
        self.assertEqual(request["method"], "fea_review")
        self.assertEqual(request["params"]["expected_revision"], 1)
        self.assertEqual(request["params"]["mesh_levels"], levels)
        self.assertEqual(request["params"]["solve_settings"]["maximum_nodes"], 256)
        self.assertTrue(request["params"]["confirmed"])
        with self.assertRaises(ValueError):
            doc.fea_review(
                2, 33, 7, "pressure-case", material={},
                constrained_face_ordinals=[0], face_tractions=[{}],
                mesh_levels=levels[:1], confirmed=True,
            )

    def test_local_pdm_helpers_are_guarded_and_keep_confirmation_explicit(self):
        process = FakeProcess()
        doc = self.session(process).new_document()
        dependencies = [{"logical_path": "supplier/bearing.step",
                         "source_path": "bearing.step"}]
        doc.pdm_release(
            "local-pdm", dependencies=dependencies, actor="designer",
            created_unix_ms=1_700_000_000_000, note="reviewed root", confirmed=True,
        )
        request = process.requests[-1]
        self.assertEqual(request["method"], "pdm_release_create")
        self.assertEqual(request["params"]["expected_revision"], 1)
        self.assertEqual(request["params"]["dependencies"], dependencies)
        self.assertIsNone(request["params"]["parent_release_id"])
        self.assertTrue(request["params"]["confirmed"])

        doc.pdm_open_release("local-pdm", "a" * 64)
        self.assertEqual(process.requests[-1]["method"], "pdm_release_open")
        doc.pdm_catalog("local-pdm")
        self.assertEqual(process.requests[-1]["method"], "pdm_catalog")
        doc.pdm_compare("local-pdm", "a" * 64, "b" * 64)
        self.assertEqual(process.requests[-1]["method"], "pdm_compare")
        self.assertEqual(process.requests[-1]["params"]["expected_digest"], "1")

        with self.assertRaises(ValueError):
            doc.pdm_release("local-pdm", actor="", created_unix_ms=1, confirmed=True)
        with self.assertRaises(ValueError):
            doc.pdm_compare("local-pdm", "", "b" * 64)

    def test_set_color_uses_shared_apply_and_refreshes_guards(self):
        process = FakeProcess()
        doc = self.session(process).new_document()
        for color in [(0, 128, 255), None]:
            before = process.revision
            doc.set_color([22, 23], color)
            request = process.requests[-1]
            self.assertEqual(request["method"], "apply")
            self.assertEqual(request["params"]["expected_revision"], before)
            self.assertEqual(request["params"]["program"]["operations"], [{
                "operation": "set_color", "selector": {"type": "occurrences", "occurrence_ids": [22, 23]},
                "color": None if color is None else list(color)}])

    def test_old_handle_invalid_after_replacement(self):
        process = FakeProcess()
        session = self.session(process)
        old = session.new_document()
        session.open_document("model.ketchup", discard_unsaved=True)
        with self.assertRaises(SessionClosedError):
            old.save("other.ketchup")

    def test_ambiguous_replacement_expires_handles_until_explicit_recovery(self):
        for method in ("new", "open"):
            for compact in (False, True):
                for applied in (False, True):
                    with self.subTest(method=method, compact=compact, applied=applied):
                        process = FakeProcess()
                        with self.session(process, compact=compact) as session:
                            old = session.new_document()
                            old.box("unsaved", 10, 20, 3)
                            old_state = dict(session._state)
                            details = {"mutation_outcome": "possibly_applied"}

                            def ambiguous(req):
                                if req["method"] == method:
                                    if applied:
                                        process.revision += 1
                                    return (json.dumps({"protocol": PROTOCOL, "id": req["id"], "error": {
                                        "code": "output_too_large", "message": "response exceeds 4 MiB",
                                        "details": details}}) + "\n").encode()
                                return process.normal(req)

                            process.answer = ambiguous
                            replace = session.new_document if method == "new" else lambda **kw: session.open_document("next.ketchup", **kw)
                            count = len(process.requests)
                            with self.assertRaises(HeadlessError) as caught:
                                replace(discard_unsaved=True)
                            self.assertEqual(caught.exception.code, "output_too_large")
                            self.assertEqual(caught.exception.details, details)
                            self.assertEqual(len(process.requests), count + 1)
                            self.assertFalse(process.stopped)
                            self.assertIsNone(session._state)
                            count = len(process.requests)
                            for call in (old.summary, lambda: old.state, lambda: old.apply([]),
                                         lambda: old.save("wrong.ketchup"), old.undo):
                                with self.assertRaises(SessionClosedError):
                                    call()
                            self.assertEqual(len(process.requests), count)

                            def recovered(req):
                                # Full state can still overflow; guard recovery must use summary.
                                self.assertNotEqual(req["method"], "state")
                                response = json.loads(process.normal(req))
                                if applied:
                                    response["result"]["state"]["document_id"] = 2
                                return (json.dumps(response) + "\n").encode()

                            process.answer = recovered
                            current_revision = process.revision
                            fresh = session.new_document(discard_unsaved=False)
                            self.assertEqual(process.requests[count]["method"], "summary")
                            params = process.requests[count + 1]["params"]
                            self.assertEqual(params["expected_revision"], current_revision)
                            self.assertEqual(params["expected_digest"], str(current_revision))
                            self.assertFalse(params["discard_unsaved"])
                            self.assertEqual(fresh.summary()["state"]["document_id"], 2 if applied else old_state["document_id"])
                            with self.assertRaises(SessionClosedError):
                                old.summary()
                            fresh.apply([])

    def test_ambiguous_replacement_recovery_does_not_discard_unsaved_work(self):
        process = FakeProcess()
        with self.session(process) as session:
            old = session.new_document()
            old.box("unsaved", 10, 20, 3)

            def answer(req):
                if req["method"] in {"open", "new"}:
                    ambiguous = req["method"] == "open"
                    error = {"code": "output_too_large" if ambiguous else "unsaved_changes",
                             "message": "replacement not confirmed"}
                    if ambiguous:
                        error["details"] = {"mutation_outcome": "possibly_applied"}
                    return (json.dumps({"protocol": PROTOCOL, "id": req["id"], "error": error}) + "\n").encode()
                return process.normal(req)

            process.answer = answer
            with self.assertRaises(HeadlessError):
                session.open_document("next.ketchup", discard_unsaved=True)
            before = process.revision
            with self.assertRaises(HeadlessError) as caught:
                session.new_document()
            self.assertEqual(caught.exception.code, "unsaved_changes")
            self.assertFalse(process.requests[-1]["params"]["discard_unsaved"])
            self.assertEqual(process.revision, before)
            self.assertFalse(process.stopped)
            with self.assertRaises(SessionClosedError):
                old.summary()

    def test_definite_replacement_rejection_preserves_old_handle(self):
        for method in ("new", "open"):
            for code, details in (("unsaved_changes", None), ("stale_state", {"invariant": "revision"}),
                                  ("persistence_error", None), ("invalid_params", {"mutation_outcome": "not_applied"})):
                with self.subTest(method=method, code=code):
                    process = FakeProcess()
                    with self.session(process) as session:
                        old = session.new_document()
                        before = dict(session._state)
                        generation = session._generation

                        def reject(req):
                            return (json.dumps({"protocol": PROTOCOL, "id": req["id"], "error": {
                                "code": code, "message": "rejected before replacement", "details": details}}) + "\n").encode()

                        process.answer = reject
                        with self.assertRaises(HeadlessError):
                            if method == "new":
                                session.new_document()
                            else:
                                session.open_document("missing.ketchup")
                        self.assertEqual(session._generation, generation)
                        self.assertEqual(session._state, before)
                        self.assertFalse(process.stopped)
                        process.answer = process.normal
                        self.assertEqual(old.summary()["state"], before)
                        old.apply([])

    def test_nan_and_oversize_never_sent(self):
        process = FakeProcess()
        doc = self.session(process).new_document()
        count = len(process.requests)
        for bad in [float("nan"), float("inf"), "x" * MAX_LINE_BYTES]:
            with self.assertRaises(ValueError):
                doc.apply([{"bad": bad}])
        self.assertEqual(len(process.requests), count)

    def test_error_preserves_details_and_session(self):
        details = {"invariant": "source_revision", "hints": ["refresh"]}
        def answer(req):
            return (json.dumps({"protocol": PROTOCOL, "id": req["id"], "error":
                               {"code": "stale_state", "message": "stale", "details": details}}) + "\n").encode()
        session = self.session(FakeProcess(answer))
        with self.assertRaises(HeadlessError) as caught:
            session.capabilities()
        self.assertEqual(caught.exception.code, "stale_state")
        self.assertEqual(caught.exception.details, details)
        self.assertFalse(session._closed)

    def test_bad_responses_close(self):
        responses = [b"not json\n", b"{}\n", b'{"protocol":"wrong","id":1,"result":{}}\n',
                     b'{"protocol":"ketchup.headless.v1","id":2,"result":{}}\n',
                     b'{"protocol":"ketchup.headless.v1","id":1,"result":{"x":NaN}}\n',
                     b'{"protocol":"ketchup.headless.v1","id":1,"result":{"x":1e999}}\n',
                     b'{"protocol":"ketchup.headless.v1","id":1,"id":1,"result":{}}\n',
                     b"x" * (MAX_LINE_BYTES + 1)]
        for response in responses:
            with self.subTest(response=response[:80]):
                session = self.session(FakeProcess(lambda req: response))
                with self.assertRaises(ProtocolError):
                    session.capabilities()
                self.assertTrue(session._closed)

    def test_timeout_closes_and_reaps(self):
        process = FakeProcess(lambda req: None)
        session = self.session(process, timeout=0.02)
        with self.assertRaises(TransportTimeout):
            session.capabilities()
        self.assertTrue(process.stopped and process.waited)

    def test_stderr_is_drained_bounded_and_escaped(self):
        process = FakeProcess()
        session = self.session(process)
        process.stderr.q.put(b"x" * 100000 + b"\x1b[2J")
        # A deterministic end marker after the data, not a sleep-based assertion.
        process.stderr.q.put(None)
        session._threads[1].join(timeout=2)
        self.assertLessEqual(len(session._stderr), 65536)
        self.assertNotIn("\x1b", session.stderr)
        self.assertIn("\\u001b", session.stderr)

    def test_batch_is_one_request_and_no_id_fabrication(self):
        process = FakeProcess()
        doc = self.session(process).new_document()
        operations = [{"operation": "copy", "selector": {"type": "occurrences", "occurrence_ids": [5]},
                       "translation_mm": [10, 0, 0]}] * 2
        doc.apply(operations)
        self.assertEqual(process.requests[-1]["params"]["program"], {"operations": operations})
        self.assertEqual(sum(r["method"] == "apply" for r in process.requests), 1)


if __name__ == "__main__":
    unittest.main()
