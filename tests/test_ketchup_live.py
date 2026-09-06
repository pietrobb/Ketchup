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
import sys
import threading
import time
import traceback
from unittest.mock import patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "sdk" / "python"))
from ketchup import LiveSession, Session, SessionClosedError
from ketchup.live import (
    MAX_FRAME_BYTES, LiveBridgeError, LiveProtocolError, LiveTimeout,
    LiveTransportError, Stamp, save_image, MAX_PNG_BYTES,
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
            "TransportError", "TransportTimeout", "rectangle", "LiveSession"} <= set(ketchup.__all__)


def test_all_methods_match_wire_and_do_not_refresh_expected_or_modify_inputs():
    def answer(req, stream):
        if req["request"]["method"] == "image":
            return response(req, error="unsupported_image")
        return response(req, result={"disconnected": True} if req["request"]["method"] == "disconnect" else {},
                        stamp=Stamp(7, 99, "b" * 64, 101))
    expected, program, selection = asdict(STAMP), copy.deepcopy(PROGRAM), [1]
    original = copy.deepcopy((expected, program, selection))
    with Peer(answer) as peer, LiveSession(peer.address, TOKEN) as live:
        assert peer.requests == []  # No implicit handshake/status/refresh.
        assert live.status()["stamp"]["mutation_epoch"] == 101
        live.summary()
        live.query(expected, kind="instances", limit=2, search="Č", definition_id=3,
                   tag_id=7, classification_dimension_id=9, classification_category_id=10,
                   cursor="opaque", world_bounds_mm=[[-1, -2, -3], [4, 5, 6]])
        live.detail(STAMP, "features", 5)
        live.propose(expected, selection, program)
        live.commit(expected, 9)
        live.undo(expected)
        live.redo(expected)
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
        "status", "summary", "query", "detail", "propose", "commit", "undo",
        "redo", "selection", "view", "image", "disconnect"]
    assert [req["id"] for req in peer.requests] == list(range(1, 13))
    assert requests[2] == {"method": "query", "expected": expected, "query": {
        "kind": "instances", "limit": 2, "search": "Č", "definition_id": 3,
        "tag_id": 7, "classification_dimension_id": 9, "classification_category_id": 10,
        "cursor": "opaque", "world_bounds_mm": [[-1, -2, -3], [4, 5, 6]]}}
    assert requests[3] == {"method": "detail", "expected": expected, "kind": "features", "entity_id": 5}
    assert requests[4] == {"method": "propose", "expected": expected, "selection": [1], "program": program}
    assert requests[5] == {"method": "commit", "expected": expected, "proposal_id": 9}
    assert requests[8]["occurrence_ids"] == [1] and requests[9]["view"] == "zoom_fit"
    assert requests[10]["capture_mode"] == "offscreen"
    assert all(req["expected"] == expected for req in requests[2:11])
    assert (expected, program, selection) == original
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


def test_unrecognized_error_text_is_not_exposed():
    with Peer(lambda req, stream: response(req, error="sensitive server message")) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveBridgeError) as caught:
            live.status()
        assert caught.value.code == "remote_error"
        assert "sensitive server message" not in repr(caught.value)
        assert live.closed


def test_response_limit_after_mutation_is_unknown():
    with Peer(lambda req, stream: response(req, error="response_limit", stamp=None)) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveProtocolError) as caught:
            live.commit(STAMP, 1)
        assert caught.value.mutation_outcome_unknown and live.closed
        assert len(peer.requests) == 1


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


def image_response(data=None, width=2, height=2, capture_mode="offscreen"):
    data = png_fixture(width, height) if data is None else data
    visible = capture_mode == "visible_viewport"
    return response({"id": 1}, result={"data": base64.b64encode(data).decode("ascii"),
        "width": width, "height": height, "byte_count": len(data),
        "stamp": asdict(STAMP), "capture_stamp": asdict(STAMP), "capture_id": 21, "render_id": 34,
        "mime_type": "image/png", "encoding": "base64", "scope": "cad_viewport",
        "capture_mode": capture_mode, "capture_pass": 34,
        "render": {"render_correlated": True, "callback_correlated": not visible,
                   "viewport_visibility_required": visible, "viewport_unoccluded": visible},
        "source": "cad_viewport", "camera": {"view": "iso"}})


def test_image_artifact_preserves_metadata_and_original_hash(tmp_path):
    value = image_response()
    original = copy.deepcopy(value)
    destination = tmp_path / "new" / "capture.png"
    with Peer(lambda req, stream: {**value, "id": req["id"]}) as peer, LiveSession(peer.address, TOKEN) as live:
        receipt = save_image(live.image(STAMP), STAMP, str(destination))
        assert [r["request"] for r in peer.requests] == [{
            "method": "image", "expected": asdict(STAMP), "capture_mode": "offscreen"}]
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


def test_visible_image_mode_is_bound_to_request_and_visibility_proof(tmp_path):
    value = image_response(capture_mode="visible_viewport")
    destination = tmp_path / "visible.png"
    with Peer(lambda req, stream: {**value, "id": req["id"]}) as peer, LiveSession(
            peer.address, TOKEN) as live:
        receipt = save_image(
            live.image(STAMP, "visible_viewport"), STAMP, str(destination), "visible_viewport")
        assert peer.requests[0]["request"]["capture_mode"] == "visible_viewport"
    assert receipt["result"]["capture_mode"] == "visible_viewport"
    assert receipt["result"]["render"]["viewport_unoccluded"] is True
    mismatched = tmp_path / "mismatched.png"
    with pytest.raises(LiveProtocolError):
        save_image(value, STAMP, str(mismatched), "offscreen")
    assert not mismatched.exists()


@pytest.mark.parametrize("field", list(asdict(STAMP)))
@pytest.mark.parametrize("location", ["stamp", "capture_stamp"])
def test_image_tampered_stamp_never_writes(tmp_path, field, location):
    value = image_response()
    stamp = value["stamp"] if location == "stamp" else value["result"][location]
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
    lambda r: r.update(byte_count=True), lambda r: r.update(byte_count=MAX_PNG_BYTES + 1),
    lambda r: r.update(byte_count=r["byte_count"] + 1),
    lambda r: r.pop("stamp"), lambda r: r.update(artifact={}),
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
    original = Path.open
    def raced(path, mode="r", *args, **kwargs):
        if path == target and mode == "xb":
            with original(path, "wb") as stream:
                stream.write(b"concurrent-owner")
        return original(path, mode, *args, **kwargs)
    monkeypatch.setattr(Path, "open", raced)
    with pytest.raises(FileExistsError):
        save_image(image_response(), STAMP, str(target))
    assert target.read_bytes() == b"concurrent-owner"


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
    pixels = b"\0" + random.Random(42).randbytes(18000)
    data = png_fixture(60, 100, b"".join(b"\0" + pixels[1 + i * 180:1 + (i + 1) * 180] for i in range(100)))
    value = image_response(data, 60, 100)
    assert 24000 < len(json.dumps(value).encode()) < MAX_FRAME_BYTES
    # Metadata padding must stay bounded even with a full payload.
    value["result"]["render_metadata"] = "r" * 1000
    receipt = save_image(value, STAMP, str(tmp_path / "bounded.png"))
    assert len(json.dumps(receipt).encode()) < 4096
    assert "data" not in receipt["result"]


def test_image_observed_rust_schema_without_byte_count(tmp_path):
    value = image_response()
    value["result"].pop("byte_count")
    value["result"].pop("capture_stamp")
    value["result"].update(source_size_px=[1920, 1080], crop_px=[10, 10, 640, 480],
                           pixels_per_point=1.0, sampling="nearest_center", thumbnail=True,
                           selection=[], view={"projection": "Perspective", "yaw": 0.2})
    receipt = save_image(value, STAMP, str(tmp_path / "rust-schema.png"))
    assert receipt["result"]["stamp"] == asdict(STAMP)
    assert receipt["result"]["artifact"]["byte_count"] == len(png_fixture())
    assert receipt["result"]["render"] == value["result"]["render"]


@pytest.mark.parametrize("code", ["image_unavailable", "image_timeout", "hidden_viewport", "stale_image",
    "occluded_viewport", "invalid_image_callback", "invalid_image_dimensions", "incomplete_image",
    "unsupported_image_texture", "unsupported_image_renderer"])
def test_observed_rust_image_errors_are_safe_and_nonfatal(code):
    with Peer(lambda req, stream: response(req, error=code)) as peer, LiveSession(peer.address, TOKEN) as live:
        with pytest.raises(LiveBridgeError) as caught:
            live.image(STAMP)
        assert caught.value.code == code and not live.closed
        assert len(peer.requests) == 1
