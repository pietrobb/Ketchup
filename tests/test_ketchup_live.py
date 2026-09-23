"""Local TCP contract tests only; no production attach or geometry evidence."""
import base64
import copy
import hashlib
import zlib
from dataclasses import FrozenInstanceError, asdict
import json
import random
from pathlib import Path
import secrets
import socket
import struct
import subprocess
import sys
import threading
import time
import traceback
from unittest.mock import patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "sdk" / "python"))
from ketchup import LiveSession, Session, SessionClosedError
import ketchup.live as live_module
from ketchup.live import (
    MAX_FRAME_BYTES, LiveBridgeError, LiveConsentError, LiveProtocolError, LiveTimeout,
    LiveTransportError, Stamp, save_image, MAX_PNG_BYTES, _attach_live_instance,
    _list_live_instances,
)

TOKEN = secrets.token_hex(32)
STAMP = Stamp(7, 12, "a" * 64, 24)
PROGRAM = {"operations": [{"operation": "set_color", "selector": {
    "type": "occurrences", "occurrence_ids": [1]}, "color": [1, 2, 3]}]}


def response(request, *, result=None, error=None, stamp=STAMP):
    return {"version": 1, "id": request["id"], "ok": error is None,
            "stamp": asdict(stamp) if stamp is not None else None,
            "result": ({} if result is None else result) if error is None else None,
            "error": error}


def image_capability():
    return {"image_protocol": {"version": 4,
            "capabilities": ["capture_mode", "capture_metadata", "render_metadata", "variable_size", "selection_framing", "detail_selection_framing"],
            "capture_modes": ["offscreen", "visible_viewport"],
            "default_capture_mode": "offscreen", "framing_modes": ["viewport", "selection", "detail_selection"],
            "default_framing": "viewport", "min_side_px": 512,
            "max_side_px": 1600, "default_side_px": 512}}


def frame(value):
    body = value if isinstance(value, bytes) else json.dumps(value).encode("utf-8")
    return struct.pack("!I", len(body)) + body


def read_exact(stream, count):
    data = bytearray()
    while len(data) < count:
        part = stream.recv(count - len(data))
        if not part:
            raise EOFError
        data.extend(part)
    return bytes(data)


class Peer:
    """Fake host accepting actual TCP frames with the Rust envelope shape."""

    def __init__(self, answer=None):
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(2)
        self.listener.settimeout(2)
        self.address = self.listener.getsockname()
        self.answer = answer or (lambda req, stream: response(req))
        self.requests = []
        self.errors = []
        self.closed = threading.Event()
        self.received = threading.Event()
        self.stream = None
        self.worker = threading.Thread(target=self.run, daemon=True)
        self.worker.start()

    def run(self):
        try:
            self.stream, _ = self.listener.accept()
            with self.stream as stream:
                stream.settimeout(2)
                while True:
                    size = struct.unpack("!I", read_exact(stream, 4))[0]
                    assert 1 <= size <= MAX_FRAME_BYTES
                    req = json.loads(read_exact(stream, size).decode("utf-8"))
                    assert set(req) == {"version", "id", "token", "request"}
                    assert req["version"] == 1 and type(req["id"]) is int
                    assert req["token"] == TOKEN
                    self.requests.append(req)
                    self.received.set()
                    result = self.answer(req, stream)
                    if result is not None:
                        stream.sendall(frame(result))
                    if req["request"]["method"] == "disconnect":
                        break
        except (EOFError, ConnectionError, OSError):
            pass
        except BaseException as error:
            self.errors.append(error)
        finally:
            self.closed.set()

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        if self.stream is not None:
            try:
                self.stream.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.listener.close()
        self.worker.join(3)
        assert not self.worker.is_alive()
        assert not self.errors


def test_offline_exports_preserved():
    import ketchup
    assert ketchup.Session is Session
    assert {"Session", "Document", "HeadlessError", "ProtocolError", "SessionClosedError",
            "TransportError", "TransportTimeout", "rectangle", "LiveConsentError",
            "LiveSession", "attach_live_instance", "list_live_instances"} <= set(ketchup.__all__)


def test_discovery_lists_only_nonce_verified_live_registry_entries(tmp_path):
    instance_id = "1" * 32
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    errors = []

    def broker():
        try:
            stream, _ = listener.accept()
            with stream:
                request = bytearray()
                while not request.endswith(b"\n"):
                    request.extend(stream.recv(1))
                value = json.loads(request)
                assert value["version"] == 1
                assert value["action"] == "list"
                assert set(value) == {"version", "action", "nonce"}
                assert len(value["nonce"]) == 64
                response = {"version": 1, "nonce": value["nonce"], "status": "available",
                            "instance_id": instance_id, "document": "part.ketchup"}
                stream.sendall(json.dumps(response, separators=(",", ":")).encode() + b"\n")
        except BaseException as error:
            errors.append(error)

    worker = threading.Thread(target=broker, daemon=True)
    worker.start()
    good = {"version": 1, "instance_id": instance_id,
            "consent_address": f"127.0.0.1:{listener.getsockname()[1]}"}
    (tmp_path / f"{instance_id}.json").write_text(json.dumps(good), encoding="utf-8")
    (tmp_path / f"{'2' * 32}.json").write_text(
        json.dumps({"version": 1, "instance_id": "2" * 32,
                    "consent_address": "localhost:9"}), encoding="utf-8")
    (tmp_path / "not-an-instance.json").write_text("DO_NOT_EXPOSE", encoding="utf-8")
    try:
        result = _list_live_instances(tmp_path, timeout=1)
    finally:
        listener.close()
        worker.join(3)
    assert not worker.is_alive() and not errors
    assert result == [{"instance_id": instance_id, "document": "part.ketchup",
                       "status": "available"}]
    assert "127.0.0.1" not in json.dumps(result)


def test_discovery_malformed_prefix_does_not_hide_valid_instance(tmp_path):
    instance_id = "3" * 32
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    errors = []

    def broker():
        try:
            stream, _ = listener.accept()
            with stream:
                request = bytearray()
                while not request.endswith(b"\n"):
                    request.extend(stream.recv(1))
                value = json.loads(request)
                response_value = {
                    "version": 1,
                    "nonce": value["nonce"],
                    "status": "available",
                    "instance_id": instance_id,
                    "document": "visible.ketchup",
                }
                stream.sendall(json.dumps(response_value, separators=(",", ":")).encode() + b"\n")
        except OSError:
            pass
        except BaseException as error:
            errors.append(error)

    worker = threading.Thread(target=broker, daemon=True)
    worker.start()
    entry = {"version": 1, "instance_id": instance_id,
             "consent_address": f"127.0.0.1:{listener.getsockname()[1]}"}
    valid_path = tmp_path / f"{instance_id}.json"
    valid_path.write_text(json.dumps(entry), encoding="utf-8")
    malformed = [tmp_path / f"malformed-{index}.json"
                 for index in range(live_module.MAX_DISCOVERY_INSTANCES)]
    try:
        with patch.object(Path, "iterdir", return_value=iter([*malformed, valid_path])):
            result = _list_live_instances(tmp_path, timeout=1)
    finally:
        listener.close()
        worker.join(3)
    assert not worker.is_alive() and not errors
    assert result == [{"instance_id": instance_id, "document": "visible.ketchup",
                       "status": "available"}]


def test_discovery_uses_one_global_timeout_budget(tmp_path):
    candidates = [tmp_path / f"{index:032x}.json" for index in range(8)]
    broker_timeouts = []

    def timeout_broker(_endpoint, _action, _instance_id, _nonce, timeout):
        broker_timeouts.append(timeout)
        time.sleep(timeout)
        raise TimeoutError

    started = time.monotonic()
    with patch.object(Path, "iterdir", return_value=iter(candidates)), \
            patch.object(live_module, "_registry_endpoint", return_value=("127.0.0.1", 1)), \
            patch.object(live_module, "_broker_exchange", side_effect=timeout_broker):
        result = _list_live_instances(tmp_path, timeout=0.02)
    elapsed = time.monotonic() - started

    assert result == []
    assert 1 <= len(broker_timeouts) < len(candidates)
    assert all(0 < remaining <= 0.021 for remaining in broker_timeouts)
    assert elapsed < 0.1


def test_attach_revalidates_instance_and_uses_credential_only_internally(tmp_path):
    instance_id = "3" * 32
    broker_listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    broker_listener.bind(("127.0.0.1", 0))
    broker_listener.listen(2)
    broker_listener.settimeout(2)
    requests, errors = [], []

    def broker():
        try:
            for action in ("list", "attach"):
                stream, _ = broker_listener.accept()
                with stream:
                    request = bytearray()
                    while not request.endswith(b"\n"):
                        request.extend(stream.recv(1))
                    value = json.loads(request)
                    requests.append(value)
                    assert value == {"version": 1, "action": action,
                                     "nonce": value["nonce"]}
                    assert len(value["nonce"]) == 64
                    if action == "list":
                        response_value = {"version": 1, "nonce": value["nonce"],
                                          "status": "available", "instance_id": instance_id,
                                          "document": "selected.ketchup"}
                    else:
                        response_value = {"version": 1, "nonce": value["nonce"],
                                          "status": "allowed", "instance_id": instance_id,
                                          "live_bridge_address": f"127.0.0.1:{peer.address[1]}",
                                          "token": TOKEN}
                    stream.sendall(json.dumps(response_value, separators=(",", ":")).encode() + b"\n")
        except BaseException as error:
            errors.append(error)

    entry = {"version": 1, "instance_id": instance_id,
             "consent_address": f"127.0.0.1:{broker_listener.getsockname()[1]}"}
    (tmp_path / f"{instance_id}.json").write_text(json.dumps(entry), encoding="utf-8")
    with Peer() as peer:
        worker = threading.Thread(target=broker, daemon=True)
        worker.start()
        try:
            with _attach_live_instance(instance_id, tmp_path, 1, 1) as live:
                assert live.status()["ok"]
                assert TOKEN not in repr(live)
        finally:
            broker_listener.close()
            worker.join(3)
    assert not worker.is_alive() and not errors
    assert [request["action"] for request in requests] == ["list", "attach"]
    assert all("token" not in request and "address" not in request for request in requests)


def test_attach_rejection_never_constructs_session_or_exposes_credential(tmp_path):
    instance_id = "4" * 32
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    listener.listen(2)

    def broker():
        for action in ("list", "attach"):
            stream, _ = listener.accept()
            with stream:
                request = bytearray()
                while not request.endswith(b"\n"):
                    request.extend(stream.recv(1))
                value = json.loads(request)
                response_value = {"version": 1, "nonce": value["nonce"],
                                  "status": "available" if action == "list" else "rejected",
                                  "instance_id": instance_id}
                if action == "list":
                    response_value["document"] = "part.ketchup"
                stream.sendall(json.dumps(response_value, separators=(",", ":")).encode() + b"\n")

    entry = {"version": 1, "instance_id": instance_id,
             "consent_address": f"127.0.0.1:{listener.getsockname()[1]}"}
    (tmp_path / f"{instance_id}.json").write_text(json.dumps(entry), encoding="utf-8")
    worker = threading.Thread(target=broker, daemon=True)
    worker.start()
    try:
        with pytest.raises(LiveConsentError) as caught:
            _attach_live_instance(
                instance_id, tmp_path, 1, 1,
                session_factory=lambda *args, **kwargs: pytest.fail("rejected attach constructed a session"),
            )
        assert caught.value.code == "consent_rejected"
        assert "token" not in repr(caught.value)
    finally:
        listener.close()
        worker.join(3)
    assert not worker.is_alive()


def test_relation_query_is_forwarded_and_numeric_detail_is_rejected():
    with Peer() as peer, LiveSession(peer.address, TOKEN) as live:
        live.query(STAMP, kind="relations", limit=7, search="assembly_mate",
                   definition_id=3)
        assert peer.requests[-1]["request"] == {
            "method": "query", "expected": asdict(STAMP), "query": {
                "kind": "relations", "limit": 7, "search": "assembly_mate",
                "definition_id": 3}}
        with pytest.raises(ValueError, match="invalid entity kind for numeric detail"):
            live.detail(STAMP, "relations", 1)
        live.disconnect()


def test_topology_queries_and_details_are_forwarded_with_the_same_stamp():
    with Peer() as peer, LiveSession(peer.address, TOKEN) as live:
        for kind in ("faces", "edges"):
            live.query(STAMP, kind=kind, limit=1, definition_id=3)
            live.detail(STAMP, kind, 7)
        live.disconnect()
    assert [request["request"] for request in peer.requests[:-1]] == [
        {"method": "query", "expected": asdict(STAMP), "query": {
            "kind": "faces", "limit": 1, "search": "", "definition_id": 3}},
        {"method": "detail", "expected": asdict(STAMP), "kind": "faces", "entity_id": 7},
        {"method": "query", "expected": asdict(STAMP), "query": {
            "kind": "edges", "limit": 1, "search": "", "definition_id": 3}},
        {"method": "detail", "expected": asdict(STAMP), "kind": "edges", "entity_id": 7},
    ]


def test_all_methods_match_wire_and_do_not_refresh_expected_or_modify_inputs():
    def answer(req, stream):
        method = req["request"]["method"]
        if method == "image":
            return response(req, error="unsupported_image")
        result = ({"disconnected": True} if method == "disconnect"
                  else image_capability() if method == "status" else {})
        return response(req, result=result, stamp=Stamp(7, 99, "b" * 64, 101))
    expected, program, selection = asdict(STAMP), copy.deepcopy(PROGRAM), [1]
    operation = {"type": "set_color", "color": [10, 20, 30]}
    original = copy.deepcopy((expected, program, selection, operation))
    with Peer(answer) as peer, LiveSession(peer.address, TOKEN) as live:
        assert peer.requests == []  # No implicit handshake/status/refresh.
        assert live.status()["stamp"]["mutation_epoch"] == 101
        live.summary()
        live.query(expected, kind="instances", limit=2, search="Č", definition_id=3,
                   tag_id=7, classification_dimension_id=9, classification_category_id=10,
                   cursor="opaque", world_bounds_mm=[[-1, -2, -3], [4, 5, 6]])
        live.create_workset(expected, kind="instances", limit=2, search="Č", definition_id=3,
                            tag_id=7, classification_dimension_id=9, classification_category_id=10,
                            world_bounds_mm=[[-1, -2, -3], [4, 5, 6]])
        live.workset_status(expected, "opaque-workset")
        live.start_batch_job(expected, "opaque-workset", operation)
        live.batch_job_status(expected, "opaque-job")
        live.step_batch_job(expected, "opaque-job")
        live.cancel_batch_job(expected, "opaque-job")
        live.detail(STAMP, "features", 5)
        live.propose(expected, selection, program)
        live.commit(expected, 9)
        live.undo(expected)
        live.redo(expected)
        live.save(expected)
        live.save_as(expected, str(Path(__file__).resolve().parents[1] / "save-as.ketchup"))
        live.open(expected, str(Path(__file__).resolve()))
        live.selection(expected, selection)
        live.view(expected, "zoom_fit")
        with pytest.raises(LiveBridgeError, match="unsupported_image"):
            live.image(expected)
        assert not live.closed
        assert live.disconnect()["result"] == {"disconnected": True}
        assert live.closed and live.disconnect() is None
        assert not live._token
    requests = [req["request"] for req in peer.requests]
    assert [req["method"] for req in requests] == [
        "status", "summary", "query", "workset_create", "workset_status",
        "batch_job_start", "batch_job_status", "batch_job_step", "batch_job_cancel",
        "detail", "propose", "commit", "undo", "redo", "save", "save_as", "open", "selection", "view", "image",
        "disconnect"]
    assert [req["id"] for req in peer.requests] == list(range(1, 22))
    assert requests[2] == {"method": "query", "expected": expected, "query": {
        "kind": "instances", "limit": 2, "search": "Č", "definition_id": 3,
        "tag_id": 7, "classification_dimension_id": 9, "classification_category_id": 10,
        "cursor": "opaque", "world_bounds_mm": [[-1, -2, -3], [4, 5, 6]]}}
    assert requests[3] == {"method": "workset_create", "expected": expected, "query": {
        "kind": "instances", "limit": 2, "search": "Č", "definition_id": 3,
        "tag_id": 7, "classification_dimension_id": 9, "classification_category_id": 10,
        "world_bounds_mm": [[-1, -2, -3], [4, 5, 6]]}}
    assert requests[4] == {"method": "workset_status", "expected": expected,
                           "handle": "opaque-workset"}
    assert requests[5] == {"method": "batch_job_start", "expected": expected,
                           "workset_handle": "opaque-workset", "operation": operation}
    assert requests[6] == {"method": "batch_job_status", "expected": expected,
                           "handle": "opaque-job"}
    assert requests[7] == {"method": "batch_job_step", "expected": expected,
                           "handle": "opaque-job"}
    assert requests[8] == {"method": "batch_job_cancel", "expected": expected,
                           "handle": "opaque-job"}
    assert requests[9] == {"method": "detail", "expected": expected, "kind": "features", "entity_id": 5}
    assert requests[10] == {"method": "propose", "expected": expected, "selection": [1], "program": program}
    assert requests[11] == {"method": "commit", "expected": expected, "proposal_id": 9}
    assert requests[14] == {"method": "save", "expected": expected}
    assert requests[15] == {"method": "save_as", "expected": expected,
                            "path": str(Path(__file__).resolve().parents[1] / "save-as.ketchup")}
    assert requests[16] == {"method": "open", "expected": expected,
                            "path": str(Path(__file__).resolve())}
    assert requests[17]["occurrence_ids"] == [1] and requests[18]["view"] == "zoom_fit"
    assert requests[19]["image_protocol_version"] == 4
    assert requests[19]["capture_mode"] == "offscreen"
    assert requests[19]["max_side_px"] == 512
    assert all(req["expected"] == expected for req in requests[2:20])
    assert (expected, program, selection, operation) == original
    with pytest.raises(FrozenInstanceError):
        STAMP.mutation_epoch = 25


def test_query_omits_absent_extension_fields_for_protocol_one_compatibility():
    with Peer() as peer, LiveSession(peer.address, TOKEN) as live:
        live.query(STAMP, kind="occurrences")
    assert peer.requests[0]["request"] == {
        "method": "query",
        "expected": asdict(STAMP),
        "query": {"kind": "occurrences", "limit": 50, "search": ""},
    }


@pytest.mark.parametrize("address", [
    ("localhost", 8), ("127.0.0.2", 8), ("0.0.0.0", 8), ("::1", 8),
    ("192.168.1.1", 8), ("127.0.0.1", 0), ("127.0.0.1", 65536),
    ("127.0.0.1", True), ("127.0.0.1", "8"), ["127.0.0.1", 8],
    "localhost:8", "http://127.0.0.1:8", "127.0.0.1:08", "127.0.0.1:+8",
    "127.0.0.1:８", "127.0.0.1:8/path", "127.0.0.1:8@host", TOKEN, None,
])
def test_invalid_endpoints_never_resolve_or_connect(address):
    with patch("ketchup.live.socket.socket") as create, patch("socket.getaddrinfo") as dns:
        with pytest.raises(ValueError) as caught:
            LiveSession(address, TOKEN)
        assert TOKEN not in str(caught.value)
        create.assert_not_called()
        dns.assert_not_called()


@pytest.mark.parametrize("token", [None, b"a" * 64, "", "a" * 63, "a" * 65, "A" * 64, "g" * 64, "é" * 64])
def test_invalid_credentials(token):
    with patch("ketchup.live.socket.socket") as create:
        with pytest.raises(ValueError):
            LiveSession(("127.0.0.1", 1), token)
        create.assert_not_called()


@pytest.mark.parametrize("timeout", [0, -1, True, "1", None, float("nan"), float("inf"), 30.001, 10**1000])
def test_invalid_timeout(timeout):
    with patch("ketchup.live.socket.socket") as create:
        with pytest.raises(ValueError):
            LiveSession(("127.0.0.1", 1), TOKEN, timeout)
        create.assert_not_called()


def test_string_endpoint_no_dns_and_secret_free_repr():
    with Peer() as peer, patch("socket.getaddrinfo", side_effect=AssertionError("DNS forbidden")):
        with LiveSession(f"127.0.0.1:{peer.address[1]}", TOKEN) as live:
            assert TOKEN not in repr(live)
            live.status()
        assert TOKEN not in repr(live) and live.closed


def test_invalid_caller_parameters_are_not_sent():
    invalid_stamps = [{}, {**asdict(STAMP), "extra": 1}, {**asdict(STAMP), "mutation_epoch": True},
                      {**asdict(STAMP), "revision": -1}, {**asdict(STAMP), "document_id": 2**64},
                      {**asdict(STAMP), "canonical_digest": 3}]
    with Peer() as peer, LiveSession(peer.address, TOKEN) as live:
        for expected in invalid_stamps:
            with pytest.raises(ValueError):
                live.undo(expected)
        calls = [lambda: live.commit(STAMP, True), lambda: live.detail(STAMP, "all", 1),
                 lambda: live.detail(STAMP, "features", 0), lambda: live.view(STAMP, "shutdown"),
                 lambda: live.selection(STAMP, [1, 1]), lambda: live.selection(STAMP, [0]),
                 lambda: live.selection(STAMP, list(range(1, 102))),
                 lambda: live.query(STAMP, kind="features", limit=101),
                 lambda: live.query(STAMP, kind="features", limit=True),
                 lambda: live.query(STAMP, kind="features", search="é" * 65),
                 lambda: live.query(STAMP, kind="features", cursor="x" * 4097),
                 lambda: live.query(STAMP, kind="definitions", definition_id=1),
                 lambda: live.query(STAMP, kind="instances", classification_category_id=1),
                 lambda: live.query(STAMP, kind="features", tag_id=1),
                 lambda: live.query(STAMP, kind="instances", tag_id=True),
                 lambda: live.query(STAMP, kind="occurrences", world_bounds_mm=[[0, 0, 0], [1, 1, 1]]),
                 lambda: live.query(STAMP, kind="instances", world_bounds_mm=[[1, 0, 0], [0, 1, 1]]),
                 lambda: live.query(STAMP, kind="instances", world_bounds_mm=[[0, 0, 0], [10**10000, 1, 1]]),
                 lambda: live.create_workset(STAMP, kind="instances", tag_id=True),
                 lambda: live.workset_status(STAMP, "x" * 4097),
                 lambda: live.detail(STAMP, "instances", 1),
                 lambda: live.image(STAMP, "hidden"),
                 lambda: live.propose(STAMP, [], {"operations": []}),
                 lambda: live.propose(STAMP, [], {"operations": [{}] * 65}),
                 lambda: live.propose(STAMP, [], {"operations": [{}], "extra": 1})]
        for bad in [float("nan"), float("inf"), float("-inf"), "x" * MAX_FRAME_BYTES,
                    "\ud800", {1: "invalid key"}, object()]:
            calls.append(lambda bad=bad: live.propose(STAMP, [], {"operations": [{"bad": bad}]}))
        cyclic = {}
        cyclic["cycle"] = cyclic
        calls.append(lambda: live.propose(STAMP, [], {"operations": [cyclic]}))
        for call in calls:
            with pytest.raises(ValueError):
                call()
        assert not peer.requests and not live.closed
        live.status()
        assert len(peer.requests) == 1


@pytest.mark.parametrize("alter", [
    lambda r: {**r, "version": 2}, lambda r: {**r, "version": True},
    lambda r: {**r, "id": 99}, lambda r: {**r, "id": True},
    lambda r: {**r, "ok": 1}, lambda r: {**r, "result": []},
    lambda r: {**r, "stamp": None}, lambda r: {**r, "stamp": {"revision": 0}},
    lambda r: {**r, "stamp": {**r["stamp"], "mutation_epoch": -1}},
    lambda r: {**r, "stamp": {**r["stamp"], "mutation_epoch": 1.0}},
    lambda r: {**r, "extra": True}, lambda r: {k: v for k, v in r.items() if k != "error"},
    lambda r: {**r, "error": "bad"}, lambda r: {**r, "ok": False},
    lambda r: {**r, "ok": False, "result": None, "error": {"code": "unauthorized"}},
    lambda r: [], lambda r: {**r, "result": {"x": float("nan")}},
    lambda r: {**r, "result": {"x": float("inf")}},
    lambda r: {**r, "result": {"x": "\ud800"}},
])
def test_bad_response_types_correlation_and_nonfinite_close_socket(alter):
    with Peer(lambda req, stream: alter(response(req))) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveProtocolError) as caught:
            live.commit(STAMP, 1)
        assert caught.value.mutation_outcome_unknown
        assert live.closed and not live._token
        with pytest.raises(SessionClosedError):
            live.status()
        assert len(peer.requests) == 1


@pytest.mark.parametrize("body", [
    b'{"version":1,"version":1}',
    b'{"version":1,"id":1,"ok":true,"stamp":null,"result":{"x":1,"x":2},"error":null}',
    b'{"version":1,"id":1,"ok":true,"stamp":null,"result":{"x":1e999},"error":null}',
    b'\xff', b'{} {}', b'\xef\xbb\xbf{}', b'{', b'[' * 1000 + b']' * 1000,
])
def test_strict_json(body):
    with Peer(lambda req, stream: body) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveProtocolError):
            live.status()
        assert live.closed


@pytest.mark.parametrize("size", [0, MAX_FRAME_BYTES + 1, 2**32 - 1])
def test_bad_frame_lengths_rejected_before_body(size):
    def answer(req, stream):
        stream.sendall(struct.pack("!I", size))
    with Peer(answer) as peer, LiveSession(peer.address, TOKEN, timeout=0.5) as live:
        with pytest.raises(LiveProtocolError):
            live.status()
        assert live.closed


def test_exact_max_response_and_fragmentation():
    def answer(req, stream):
        payload = json.dumps(response(req, result={"padding": ""})).encode()
        payload = payload.replace(b'"padding": ""', b'"padding": "' + b'x' * (MAX_FRAME_BYTES - len(payload)) + b'"')
        assert len(payload) == MAX_FRAME_BYTES
        framed = frame(payload)
        for part in (framed[:1], framed[1:3], framed[3:41], framed[41:]):
            stream.sendall(part)
    with Peer(answer) as peer, LiveSession(peer.address, TOKEN) as live:
        assert live.status()["result"]["padding"]


@pytest.mark.parametrize("code", ["stale_document", "selection_changed", "unsupported_image", "unauthorized", "busy"])
def test_safe_server_rejection_no_retry(code):
    with Peer(lambda req, stream: response(req, error=code, stamp=None)) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveBridgeError) as caught:
            live.commit(STAMP, 1)
        assert caught.value.code == code
        assert live.closed == (code == "unauthorized")
        assert len(peer.requests) == 1


def test_capability_gap_exposes_only_bounded_machine_fields():
    gap = {"kind": "capability_gap",
           "capability": "planning.cad_feature_result_unsupported",
           "operation": "append_feature", "retryable": False, "published": False}

    def answer(request, _stream):
        value = response(request, error="capability_gap")
        value["result"] = gap
        return value

    with Peer(answer) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveBridgeError) as caught:
            live.apply_and_verify(PROGRAM, expected=STAMP)
        assert caught.value.code == "capability_gap"
        assert caught.value.details == gap
        assert not live.closed
        assert len(peer.requests) == 1


@pytest.mark.parametrize("where", ["error", "result", "key", "stamp", "malformed"])
def test_server_cannot_leak_token_in_return_error_repr_or_traceback(where):
    def answer(req, stream):
        if where == "error":
            return response(req, error="remote says " + TOKEN)
        if where == "result":
            return response(req, result={"message": TOKEN})
        if where == "key":
            return response(req, result={TOKEN: "value"})
        if where == "stamp":
            return response(req, stamp=Stamp(1, 1, TOKEN, 1))
        return ('{"' + TOKEN + '":broken').encode()
    with Peer(answer) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveProtocolError) as caught:
            live.status()
        rendered = "".join(traceback.format_exception(type(caught.value), caught.value, caught.value.__traceback__))
        assert TOKEN not in rendered + str(caught.value) + repr(caught.value) + repr(live)
        assert caught.value.__context__ is None
        assert live.closed and not live._token


def test_output_too_large_is_typed_and_nonfatal():
    with Peer(lambda req, stream: response(req, error="output_too_large")) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveBridgeError) as caught:
            live.status()
        assert caught.value.code == "output_too_large"
        assert not live.closed


def test_recovery_rejected_is_typed_nonfatal_and_connection_is_reusable():
    def answer(req, _stream):
        if req["request"]["method"] == "commit":
            return response(req, error="recovery_rejected")
        return response(req)

    with Peer(answer) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveBridgeError) as caught:
            live.commit(STAMP, 1)
        assert caught.value.code == "recovery_rejected"
        assert not live.closed
        assert live.status()["ok"]
        assert [request["request"]["method"] for request in peer.requests] == ["commit", "status"]


def test_unrecognized_error_text_is_not_exposed():
    with Peer(lambda req, stream: response(req, error="sensitive server message")) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveBridgeError) as caught:
            live.status()
        assert caught.value.code == "remote_error"
        assert "sensitive server message" not in repr(caught.value)
        assert live.closed


@pytest.mark.parametrize("method", ["commit", "save", "save_as", "open"])
def test_response_limit_after_mutation_is_unknown(method):
    with Peer(lambda req, stream: response(req, error="response_limit", stamp=None)) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveProtocolError) as caught:
            if method == "commit":
                live.commit(STAMP, 1)
            elif method == "save_as":
                live.save_as(STAMP, str(Path(__file__).resolve()))
            elif method == "open":
                live.open(STAMP, str(Path(__file__).resolve()))
            else:
                live.save(STAMP)
        assert caught.value.mutation_outcome_unknown and live.closed
        assert len(peer.requests) == 1


@pytest.mark.parametrize("method", ["batch_job_start", "batch_job_cancel"])
def test_lost_batch_job_response_reports_unknown_outcome(method):
    def answer(_req, stream):
        stream.shutdown(socket.SHUT_RDWR)

    with Peer(answer) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveTransportError) as caught:
            if method == "batch_job_start":
                live.start_batch_job(STAMP, "opaque-workset", {"type": "set_color", "color": [1, 2, 3]})
            else:
                live.cancel_batch_job(STAMP, "opaque-job")
        assert caught.value.mutation_outcome_unknown and live.closed
        assert [request["request"]["method"] for request in peer.requests] == [method]


@pytest.mark.parametrize("phase", ["header", "body", "shared"])
def test_total_deadline_defeats_slow_trickle(phase):
    def answer(req, stream):
        packet = frame(response(req))
        if phase == "body":
            stream.sendall(packet[:4])
            packet = packet[4:]
        elif phase == "shared":
            time.sleep(0.08)
            stream.sendall(packet[:4])
            time.sleep(0.08)
            stream.sendall(packet[4:])
            return
        for byte in packet:
            time.sleep(0.05)
            stream.sendall(bytes([byte]))
    with Peer(answer) as peer, LiveSession(peer.address, TOKEN, timeout=0.13) as live:
        start = time.monotonic()
        with pytest.raises(LiveTimeout) as caught:
            live.undo(STAMP)
        elapsed = time.monotonic() - start
        assert 0.09 <= elapsed < 0.7
        assert caught.value.mutation_outcome_unknown and "mutation outcome unknown" in str(caught.value)
        assert live.closed and not live._token
        assert len(peer.requests) == 1


@pytest.mark.parametrize("partial", [b"", b"\0\0", struct.pack("!I", 100) + b"{}"])
def test_lost_mutation_response_closes_only_socket_never_retries(partial):
    def answer(req, stream):
        stream.sendall(partial)
        stream.shutdown(socket.SHUT_RDWR)
    with Peer(answer) as peer, patch("subprocess.Popen") as spawn:
        with LiveSession(peer.address, TOKEN) as live:
            with pytest.raises(LiveTransportError) as caught:
                live.commit(STAMP, 1)
            assert caught.value.mutation_outcome_unknown and live.closed
            assert len(peer.requests) == 1
        # Host listener remains usable: the client cannot terminate an app/process.
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
            probe.settimeout(0.5)
            probe.connect(peer.address)
        spawn.assert_not_called()


def test_context_close_sends_nothing_and_discards_secret():
    with Peer() as peer:
        with LiveSession(peer.address, TOKEN) as live:
            secret_storage = live._token
            live.status()
        live.close()
        assert secret_storage == bytearray() and live.closed
        assert peer.closed.wait(1)
        assert [r["request"]["method"] for r in peer.requests] == ["status"]


def test_disconnect_failure_still_closes_without_mutation_unknown():
    def answer(req, stream):
        stream.shutdown(socket.SHUT_RDWR)
    with Peer(answer) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveTransportError) as caught:
            live.disconnect()
        assert not caught.value.mutation_outcome_unknown
        assert live.closed and not live._token
        assert [r["request"]["method"] for r in peer.requests] == ["disconnect"]


def test_requests_serialized_with_independent_total_deadlines():
    def answer(req, stream):
        stream.setblocking(False)
        try:
            assert not stream.recv(1, socket.MSG_PEEK), "request pipelined"
        except BlockingIOError:
            pass
        finally:
            stream.settimeout(2)
        time.sleep(0.02)
        return response(req)
    with Peer(answer) as peer, LiveSession(peer.address, TOKEN, timeout=1) as live:
        errors = []
        def call():
            try:
                live.summary()
            except BaseException as error:
                errors.append(error)
        workers = [threading.Thread(target=call) for _ in range(6)]
        for worker in workers:
            worker.start()
        for worker in workers:
            worker.join(2)
        assert not errors
        assert [r["id"] for r in peer.requests] == list(range(1, 7))


def test_lock_wait_is_bounded_and_unsent_mutation_not_unknown():
    with Peer() as peer, LiveSession(peer.address, TOKEN, timeout=0.05) as live:
        live._lock.acquire()
        try:
            start = time.monotonic()
            with pytest.raises(LiveTimeout) as caught:
                live.commit(STAMP, 1)
            assert time.monotonic() - start < 0.5
            assert not caught.value.mutation_outcome_unknown
            assert not peer.requests and not live.closed
        finally:
            live._lock.release()


def test_writes_share_response_deadline():
    # Small frames fit OS buffers; wrap a real TCP socket to force short, slow writes.
    class SlowWrites:
        def __init__(self, stream):
            self.stream = stream
            self.timeouts = []
        def settimeout(self, timeout):
            self.timeouts.append(timeout)
            self.stream.settimeout(timeout)
        def send(self, data):
            time.sleep(0.025)
            return self.stream.send(data[:1])
        def close(self):
            self.stream.close()
    with Peer() as peer, LiveSession(peer.address, TOKEN, timeout=0.09) as live:
        wrapper = SlowWrites(live._socket)
        live._socket = wrapper
        start = time.monotonic()
        with pytest.raises(LiveTimeout) as caught:
            live.commit(STAMP, 1)
        assert time.monotonic() - start < 0.5
        assert caught.value.mutation_outcome_unknown and live.closed
        assert all(a > b for a, b in zip(wrapper.timeouts, wrapper.timeouts[1:]))
        assert not peer.requests


def test_connect_failure_is_sanitized_and_socket_closed():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as reserved:
        reserved.bind(("127.0.0.1", 0))
        address = reserved.getsockname()  # Bound but not listening.
        with pytest.raises(LiveTransportError) as caught:
            LiveSession(address, TOKEN, timeout=0.1)
    assert TOKEN not in repr(caught.value)
    assert caught.value.__context__ is None
    assert not caught.value.mutation_outcome_unknown


def png_fixture(width=2, height=2, pixels=None):
    """Synthetic RGB8 pixels, not a rendered CAD image or visual evidence."""
    def chunk(kind, data):
        return struct.pack("!I", len(data)) + kind + data + struct.pack("!I", zlib.crc32(kind + data))
    raw = pixels if pixels is not None else (b"\0" + b"\x12\x34\x56" * width) * height
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack("!IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def image_response(data=None, width=2, height=2, capture_mode="offscreen", max_side_px=512,
                   framing="viewport", detail=None):
    data = png_fixture(width, height) if data is None else data
    visible = capture_mode == "visible_viewport"
    return response({"id": 1}, result={"data": base64.b64encode(data).decode("ascii"),
        "width": width, "height": height, "stamp": asdict(STAMP),
        "mime_type": "image/png", "encoding": "base64", "scope": "cad_viewport",
        "image_protocol_version": 4, "capture_mode": capture_mode, "capture_pass": 34,
        "source_size_px": [width, height], "crop_px": [0, 0, width, height],
        "pixels_per_point": 1.0, "sampling": "nearest_center",
        "thumbnail": framing == "viewport", "requested_max_side_px": max_side_px,
        "framing": {"mode": framing,
                    "occurrence_ids": [detail["occurrence_id"]] if detail else ([1] if framing == "selection" else []),
                    "detail": ({"kind": detail["kind"][:-1], "entity_id": detail["entity_id"],
                                "reference_id": "a" * 64} if detail else None)},
        "view": {"projection": "Perspective", "yaw": 0.2, "pitch": 0.3,
                 "target_z_mm": 0.0, "zoom": 1.0, "pan": [0.0, 0.0], "distance_mm": 10.0},
        "selection": [],
        "render": {"render_correlated": True, "callback_correlated": not visible,
                   "viewport_visibility_required": visible, "viewport_unoccluded": visible,
                   "geometry_complete": False, "source": "isolated_cad_target",
                   "gui_overlays_included": False,
                   "completeness": "display_only_not_geometry_validation",
                   "exact_contents_stamp": 5, "topology_contents_stamp": 7,
                   "exact_evaluation_complete": True, "exact_evaluation_pending": False,
                   "scene_callbacks": 0, "paint_shape_count": 12,
                   "style": "test-camera", "theme": "Dark"}})


@pytest.mark.parametrize("capability", [
    {},
    {"image_protocol": {"version": 1, "capabilities": ["capture_mode"],
                        "capture_modes": ["offscreen"], "default_capture_mode": "offscreen"}},
    {"image_protocol": {"version": 2.0,
                        "capabilities": ["capture_mode", "capture_metadata", "render_metadata"],
                        "capture_modes": ["offscreen"], "default_capture_mode": "offscreen"}},
    {"image_protocol": {"version": 2,
                        "capabilities": ["capture_mode", "capture_metadata"],
                        "capture_modes": ["offscreen", "visible_viewport"],
                        "default_capture_mode": "offscreen"}},
])
def test_image_fails_closed_without_declared_v2_capabilities(capability):
    with Peer(lambda req, stream: response(req, result=capability)) as peer, LiveSession(
            peer.address, TOKEN) as live:
        for _ in range(2):
            with pytest.raises(LiveProtocolError, match="not negotiated"):
                live.image(STAMP)
        assert [request["request"]["method"] for request in peer.requests] == ["status"]


def test_image_negotiates_supported_mode_when_server_advertises_future_mode():
    capability = image_capability()
    capability["image_protocol"]["capture_modes"].append("future_capture")
    value = image_response()

    def answer(req, stream):
        if req["request"]["method"] == "status":
            return response(req, result=capability)
        return {**value, "id": req["id"]}

    with Peer(answer) as peer, LiveSession(peer.address, TOKEN) as live:
        assert live.image(STAMP)["result"]["capture_mode"] == "offscreen"
        assert [request["request"]["method"] for request in peer.requests] == ["status", "image"]


def test_image_fails_closed_when_requested_mode_was_not_negotiated():
    capability = image_capability()
    capability["image_protocol"]["capture_modes"] = ["offscreen"]
    with Peer(lambda req, stream: response(req, result=capability)) as peer, LiveSession(
            peer.address, TOKEN) as live:
        with pytest.raises(LiveProtocolError, match="not negotiated"):
            live.image(STAMP, "visible_viewport")
        assert [request["request"]["method"] for request in peer.requests] == ["status"]


def test_first_image_negotiation_and_capture_share_one_deadline():
    value = image_response()
    def answer(req, stream):
        time.sleep(0.09)
        if req["request"]["method"] == "status":
            return response(req, result=image_capability())
        return {**value, "id": req["id"]}
    with Peer(answer) as peer, LiveSession(peer.address, TOKEN, timeout=0.14) as live:
        with pytest.raises(LiveTimeout):
            live.image(STAMP)
        assert live.closed
        assert peer.received.wait(1)


def image_answer(value):
    def answer(req, stream):
        if req["request"]["method"] == "status":
            return response(req, result=image_capability())
        return {**value, "id": req["id"]}
    return answer


def test_image_artifact_preserves_metadata_and_original_hash(tmp_path):
    value = image_response(max_side_px=1600, framing="selection")
    original = copy.deepcopy(value)
    destination = tmp_path / "new" / "capture.png"
    with Peer(image_answer(value)) as peer, LiveSession(peer.address, TOKEN) as live:
        receipt = save_image(
            live.image(STAMP, max_side_px=1600, framing="selection"),
            STAMP,
            str(destination),
            max_side_px=1600,
            framing="selection",
        )
        assert [r["request"]["method"] for r in peer.requests] == ["status", "image"]
        assert peer.requests[1]["request"] == {
            "method": "image", "expected": asdict(STAMP), "image_protocol_version": 4,
            "capture_mode": "offscreen", "max_side_px": 1600, "framing": "selection",
            "detail_target": None}
    assert value == original
    assert destination.read_bytes() == png_fixture()
    assert receipt["stamp"] == original["stamp"]
    assert {k: v for k, v in receipt["result"].items() if k != "artifact"} == {
        k: v for k, v in original["result"].items() if k != "data"}
    artifact = receipt["result"]["artifact"]
    assert artifact == {"path": str(destination), "byte_count": len(png_fixture()),
        "sha256": hashlib.sha256(png_fixture()).hexdigest(), "artifact_saved": True,
        "visual_delivery": "unverified", "geometry_evaluated": False}
    assert "data" not in receipt["result"]
    assert len(json.dumps(receipt).encode()) < MAX_FRAME_BYTES


def test_detail_selection_image_binds_host_topology_target_on_wire_and_response():
    detail = {"occurrence_id": 7, "kind": "edges", "entity_id": 73}
    value = image_response(framing="detail_selection", detail=detail)
    with Peer(image_answer(value)) as peer, LiveSession(peer.address, TOKEN) as live:
        result = live.image(
            STAMP,
            framing="detail_selection",
            detail_occurrence_id=7,
            detail_kind="edges",
            detail_entity_id=73,
        )
        assert result["result"]["framing"]["detail"]["reference_id"] == "a" * 64
        assert peer.requests[1]["request"] == {
            "method": "image",
            "expected": asdict(STAMP),
            "image_protocol_version": 4,
            "capture_mode": "offscreen",
            "max_side_px": 512,
            "framing": "detail_selection",
            "detail_target": detail,
        }


def test_live_image_rejects_malformed_success_and_closes_session():
    value = image_response()
    value["result"]["render"]["source"] = "desktop"
    with Peer(image_answer(value)) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveProtocolError, match="invalid live image response"):
            live.image(STAMP)
        assert live.closed
        assert [request["request"]["method"] for request in peer.requests] == ["status", "image"]


def test_visible_image_mode_is_bound_to_request_and_visibility_proof(tmp_path):
    value = image_response(capture_mode="visible_viewport")
    destination = tmp_path / "visible.png"
    with Peer(image_answer(value)) as peer, LiveSession(peer.address, TOKEN) as live:
        receipt = save_image(
            live.image(STAMP, "visible_viewport"), STAMP, str(destination), "visible_viewport")
        assert [request["request"]["method"] for request in peer.requests] == ["status", "image"]
        assert peer.requests[1]["request"]["image_protocol_version"] == 4
        assert peer.requests[1]["request"]["max_side_px"] == 512
        assert peer.requests[1]["request"]["capture_mode"] == "visible_viewport"
    assert receipt["result"]["capture_mode"] == "visible_viewport"
    assert receipt["result"]["render"]["viewport_unoccluded"] is True
    mismatched = tmp_path / "mismatched.png"
    with pytest.raises(LiveProtocolError):
        save_image(value, STAMP, str(mismatched), "offscreen")
    assert not mismatched.exists()


@pytest.mark.parametrize("field", list(asdict(STAMP)))
@pytest.mark.parametrize("location", ["envelope", "result"])
def test_image_tampered_stamp_never_writes(tmp_path, field, location):
    value = image_response()
    stamp = value["stamp"] if location == "envelope" else value["result"]["stamp"]
    stamp[field] = "wrong" if field == "canonical_digest" else stamp[field] + 1
    destination = tmp_path / "absent" / "bad.png"
    with pytest.raises(LiveProtocolError):
        save_image(value, STAMP, str(destination))
    assert not destination.parent.exists()


@pytest.mark.parametrize("alter", [
    lambda r: r.update(data="not a PNG"),
    lambda r: r.update(data=r["data"] + "\n"),
    lambda r: r.update(data=r["data"] + "="),
    lambda r: r.update(data="é"),
    lambda r: r.update(data="A" * (MAX_FRAME_BYTES + 4)),
    lambda r: r.update(width=True), lambda r: r.update(height=0),
    lambda r: r.update(width=2049), lambda r: r.update(width=3),
    lambda r: r.pop("source_size_px"), lambda r: r.update(sampling="bilinear"),
    lambda r: r["render"].update(source="desktop"),
    lambda r: r["render"].update(geometry_complete=True),
    lambda r: r.pop("stamp"), lambda r: r.update(artifact={}),
    lambda r: r.pop("image_protocol_version"), lambda r: r.update(image_protocol_version=1),
    lambda r: r.update(image_protocol_version=2.0),
    lambda r: r.update(mime_type="text/plain"), lambda r: r.update(scope="desktop"),
    lambda r: r.update(encoding="raw"), lambda r: r.update(capture_pass=True),
    lambda r: r.update(render={"render_correlated": False, "callback_correlated": False,
                               "viewport_visibility_required": False, "viewport_unoccluded": False}),
])
def test_image_bad_encoding_and_metadata_never_writes(tmp_path, alter):
    value = image_response()
    alter(value["result"])
    destination = tmp_path / "bad.png"
    with pytest.raises(LiveProtocolError):
        save_image(value, STAMP, str(destination))
    assert not destination.exists()


@pytest.mark.parametrize("data", [
    b"not PNG" * 10, png_fixture()[:33], png_fixture()[:-1],
    png_fixture() + b"trailing", png_fixture()[:40] + b"x" + png_fixture()[41:],
    png_fixture(pixels=b"\0"), png_fixture(pixels=b"\x05" + b"x" * 6 + b"\0" + b"x" * 6),
    png_fixture(pixels=b"x" * 1000000), png_fixture(2049, 1),
])
def test_image_malformed_png_never_writes(tmp_path, data):
    destination = tmp_path / "bad.png"
    with pytest.raises(LiveProtocolError):
        save_image(image_response(data), STAMP, str(destination))
    assert not destination.exists()


def test_image_existing_file_and_exclusive_create_race(tmp_path, monkeypatch):
    destination = tmp_path / "existing.png"
    destination.write_bytes(b"user-owned")
    with pytest.raises(FileExistsError):
        save_image(image_response(), STAMP, str(destination))
    assert destination.read_bytes() == b"user-owned"
    target = tmp_path / "race.png"
    original = live_module._open_new_image_file
    def raced(path):
        if path == target:
            target.write_bytes(b"concurrent-owner")
        return original(path)
    monkeypatch.setattr(live_module, "_open_new_image_file", raced)
    with pytest.raises(FileExistsError):
        save_image(image_response(), STAMP, str(target))
    assert target.read_bytes() == b"concurrent-owner"


@pytest.mark.skipif(sys.platform != "win32", reason="Windows junction regression")
def test_image_ancestor_cannot_be_replaced_after_validation(tmp_path, monkeypatch):
    parent = tmp_path / "checked"
    parent.mkdir()
    displaced = tmp_path / "displaced"
    outside = tmp_path / "outside"
    outside.mkdir()
    target = parent / "capture.png"
    original = live_module._windows_create_new_image_handle
    race_attempted = False

    def raced(parent_handle, name):
        nonlocal race_attempted
        race_attempted = True
        parent.rename(displaced)
        subprocess.run(
            ["cmd.exe", "/d", "/c", "mklink", "/J", str(parent), str(outside)],
            check=True,
            capture_output=True,
        )
        return original(parent_handle, name)

    monkeypatch.setattr(live_module, "_windows_create_new_image_handle", raced)
    with pytest.raises(ValueError):
        save_image(image_response(), STAMP, str(target))
    assert race_attempted
    assert not (outside / target.name).exists()
    assert not (displaced / target.name).exists()


@pytest.mark.parametrize("dangling", [False, True])
def test_image_refuses_symlink(tmp_path, dangling):
    target = tmp_path / "target.png"
    if not dangling:
        target.write_bytes(b"owned")
    link = tmp_path / "link.png"
    try:
        link.symlink_to(target)
    except OSError:
        pytest.skip("symlink creation not permitted on this host")
    with pytest.raises(ValueError):
        save_image(image_response(), STAMP, str(link))
    assert not target.exists() if dangling else target.read_bytes() == b"owned"


@pytest.mark.parametrize("path", ["", "relative.png", "C:/bad.txt", "C:/bad.png:stream", None])
def test_image_requires_explicit_absolute_png_path(path):
    with pytest.raises((ValueError, TypeError)):
        save_image(image_response(), STAMP, path)


def test_image_near_wire_limit_returns_compact_receipt(tmp_path):
    # Deterministic noisy synthetic RGB data approaches the wire budget.
    pixels = random.Random(42).randbytes(64 * 64 * 3)
    data = png_fixture(64, 64, b"".join(
        b"\0" + pixels[i * 192:(i + 1) * 192] for i in range(64)))
    value = image_response(data, 64, 64)
    assert 16000 < len(json.dumps(value).encode()) < MAX_FRAME_BYTES
    receipt = save_image(value, STAMP, str(tmp_path / "bounded.png"))
    assert len(json.dumps(receipt).encode()) < 4096
    assert "data" not in receipt["result"]


def test_image_observed_rust_v2_schema(tmp_path):
    value = image_response()
    receipt = save_image(value, STAMP, str(tmp_path / "rust-schema.png"))
    assert receipt["result"]["stamp"] == asdict(STAMP)
    assert receipt["result"]["artifact"]["byte_count"] == len(png_fixture())
    assert receipt["result"]["render"] == value["result"]["render"]


@pytest.mark.parametrize("code", ["image_unavailable", "image_timeout", "hidden_viewport", "stale_image",
    "unsupported_image_protocol", "occluded_viewport", "invalid_image_callback", "invalid_image_dimensions", "incomplete_image",
    "unsupported_image_texture", "unsupported_image_renderer"])
def test_observed_rust_image_errors_are_safe_and_nonfatal(code):
    def answer(req, stream):
        if req["request"]["method"] == "status":
            return response(req, result=image_capability())
        return response(req, error=code)
    with Peer(answer) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveBridgeError) as caught:
            live.image(STAMP)
        assert caught.value.code == code and not live.closed
        assert [request["request"]["method"] for request in peer.requests] == ["status", "image"]
