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
