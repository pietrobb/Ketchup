"""Registered .call contract/ownership tests, NOT GUI or geometry evidence."""
import asyncio
import base64
import zlib
import copy
import importlib.util
import io
import json
import os
from pathlib import Path
import socket
import struct
import sys
import threading
import time
from types import SimpleNamespace

import pytest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("tested_ketchup_live_skill", ROOT / "skills/ketchup_live.py")
skill = importlib.util.module_from_spec(spec)
spec.loader.exec_module(skill)
STAMP = {"document_id": 7, "revision": 2, "canonical_digest": "abc", "mutation_epoch": 8}
PROGRAM = {"operations": [{"operation": "contract_test_only"}]}


def envelope(result=None, stamp=None):
    return {"version": 1, "id": 1, "ok": True, "stamp": copy.deepcopy(stamp or STAMP),
            "result": result or {}, "error": None}


class SessionDouble:
    """Contract state only; no invented geometric evaluations."""
    def __init__(self):
        self.stamp = copy.deepcopy(STAMP)
        self.selected = []
        self.calls = []
        self.closed = False
        self.fail = None

    def close(self):
        self.closed = True
        self.calls.append(("close",))

    def status(self):
        self.calls.append(("status",))
        return envelope({"selection": self.selected}, self.stamp)

    def summary(self):
        return envelope({"metadata_only": True}, self.stamp)

    def request(self, method, expected, *args, **kwargs):
        self.calls.append((method, copy.deepcopy(expected), copy.deepcopy(args), copy.deepcopy(kwargs)))
        if self.fail:
            raise self.fail
        if expected != self.stamp:
            raise skill._live().LiveBridgeError("stale_document")
        if method == "image":
            raise skill._live().LiveBridgeError("unsupported_image")
        if method in ("commit", "undo", "redo"):
            self.stamp["revision"] += 1
            self.stamp["mutation_epoch"] += 1
            self.stamp["canonical_digest"] = method
        if method == "selection":
            self.selected = args[0]
        return envelope({"proposal_id": 9} if method == "propose" else {}, self.stamp)

    def __getattr__(self, method):
        if method in ("query", "detail", "create_workset", "workset_status", "propose", "commit", "undo", "redo", "selection", "view", "image"):
            return lambda expected, *args, **kwargs: self.request(method, expected, *args, **kwargs)
        raise AttributeError(method)


def tools(state, launcher):
    return {tool.name: tool for tool in skill._register_tools(state, launcher=launcher)}


async def call(registered, name, **kwargs):
    text = await registered[name].call(kwargs)
    assert isinstance(text, str)
    assert len(text.encode("utf-8")) <= 32768
    return json.loads(text)


async def launch(registered):
    result = await call(registered, "KetchupLiveSession", action="launch", executable=sys.executable)
    assert result["ok"], result
    return result


def test_registration_shared_helpers_no_offline_runtime_or_shadow(monkeypatch):
    before_path = list(sys.path)
    public = sys.modules.get("ketchup")
    monkeypatch.setattr(skill._helpers, "Runtime", lambda *args: pytest.fail("offline runtime created"))
    namespace = {"__name__": "src.claude_engine", "skill": skill}
    exec("class ClaudeEngine:\n def load(self):\n  return skill.register_tools()", namespace)
    engine = namespace["ClaudeEngine"]()
    engine._plan_state = SimpleNamespace(active=True)
    registered = {tool.name: tool for tool in engine.load()}
    assert set(registered) == {"KetchupLiveSession", "KetchupLiveInspect", "KetchupLiveEdit", "KetchupLiveView"}
    for tool in registered.values():
        schema = tool.to_dict()
        props = schema["input_schema"]["properties"]
        assert not {"token", "address", "plan_mode", "launcher", "session_factory", "args", "env"} & props.keys()
    required = registered["KetchupLiveEdit"].to_dict()["input_schema"]["required"]
    assert {"expected", "selection"} <= set(required)
    capture = registered["KetchupLiveView"].to_dict()["input_schema"]["properties"]["capture_mode"]
    assert capture["default"] == "offscreen"
    async def scenario():
        result = await call(registered, "KetchupLiveSession", action="launch", executable=sys.executable)
        assert result["error"]["code"] == "plan_mode"
    asyncio.run(scenario())
    assert skill._sdk().LiveSession
    assert sys.path == before_path and sys.modules.get("ketchup") is public


def test_registered_lifecycle_stamps_selection_and_stale_rejection(tmp_path, monkeypatch):
    monkeypatch.setattr(skill, "IMAGE_ROOT", tmp_path)
    session = SessionDouble()
    state = SimpleNamespace(active=False)
    registered = tools(state, lambda *args: session)
    async def scenario():
        opened = await launch(registered)
        handle = opened["result"]["handle"]
        assert opened["stamp"] == STAMP
        assert opened["result"]["ownership"] == "nonowning_new_GUI_window"
        original = copy.deepcopy(STAMP)
        for action in ("status", "summary", "query", "detail"):
            result = await call(registered, "KetchupLiveInspect", action=action, handle=handle,
                                expected=STAMP, kind="features", entity_id=3, cursor="opaque")
            assert result["stamp"] == STAMP
        assert session.calls[-2][3]["cursor"] == "opaque"
        bounded = await call(registered, "KetchupLiveInspect", action="query", handle=handle,
                             expected=STAMP, kind="instances", tag_id=7,
                             classification_dimension_id=9, classification_category_id=10,
                             world_bounds_mm=[[-1, -2, -3], [4, 5, 6]])
        assert bounded["stamp"] == STAMP
        assert session.calls[-1][3]["world_bounds_mm"] == [[-1, -2, -3], [4, 5, 6]]
        assert session.calls[-1][3]["tag_id"] == 7
        assert session.calls[-1][3]["classification_category_id"] == 10
        relations = await call(registered, "KetchupLiveInspect", action="query", handle=handle,
                               expected=STAMP, kind="relations", search="assembly_mate",
                               definition_id=3)
        assert relations["stamp"] == STAMP
        assert session.calls[-1][3]["kind"] == "relations"
        assert session.calls[-1][3]["definition_id"] == 3
        workset = await call(registered, "KetchupLiveInspect", action="workset_create",
                             handle=handle, expected=STAMP, kind="instances", tag_id=7)
        assert workset["stamp"] == STAMP
        assert session.calls[-1][0] == "create_workset"
        assert session.calls[-1][3]["tag_id"] == 7
        status = await call(registered, "KetchupLiveInspect", action="workset_status",
                            handle=handle, expected=STAMP, workset_handle="opaque-workset")
        assert status["stamp"] == STAMP
        assert session.calls[-1][0] == "workset_status"
        assert session.calls[-1][2] == ("opaque-workset",)
        assert (await call(registered, "KetchupLiveInspect", action="workset_create",
                           handle=handle, expected=STAMP, cursor="partial"))["error"]["code"] == "invalid_arguments"
        rejected = await call(registered, "KetchupLiveEdit", action="propose", handle=handle,
                              expected=STAMP, selection=[1], program=PROGRAM)
        assert rejected["error"]["code"] == "selection_changed"
        assert not any(c[0] == "propose" for c in session.calls)
        proposed = await call(registered, "KetchupLiveEdit", action="propose", handle=handle,
                              expected=STAMP, selection=[], program=PROGRAM)
        assert proposed["stamp"] == STAMP and proposed["result"]["proposal_id"] == 9
        result = await call(registered, "KetchupLiveEdit", action="commit", handle=handle,
                            expected=STAMP, selection=[], proposal_id=9)
        assert result["stamp"]["mutation_epoch"] == STAMP["mutation_epoch"] + 1
        assert STAMP == original
        stale = await call(registered, "KetchupLiveEdit", action="commit", handle=handle,
                           expected=STAMP, selection=[], proposal_id=9)
        assert stale["error"]["code"] == "stale_document"
        assert sum(c[0] == "commit" for c in session.calls) == 1
        for action in ("undo", "redo"):
            previous = result["stamp"]
            result = await call(registered, "KetchupLiveEdit", action=action, handle=handle,
                                expected=previous, selection=[])
            assert result["stamp"]["mutation_epoch"] == previous["mutation_epoch"] + 1
        expected = result["stamp"]
        assert (await call(registered, "KetchupLiveView", action="selection", handle=handle,
                           expected=expected, occurrence_ids=[1]))["stamp"] == expected
        assert (await call(registered, "KetchupLiveView", action="view", handle=handle,
                           expected=expected, view="iso"))["stamp"] == expected
        image = await call(registered, "KetchupLiveView", action="image", handle=handle, expected=expected,
                           image_path=str(tmp_path / "unsupported.png"))
        assert image["error"]["code"] == "unsupported_image"
        state.active = True
        disconnected = await call(registered, "KetchupLiveSession", action="disconnect", handle=handle)
        assert disconnected["result"]["app_terminated"] is False and session.closed
        assert (await call(registered, "KetchupLiveInspect", action="status", handle=handle))["error"]["code"] == "invalid_handle"
    asyncio.run(scenario())


@pytest.mark.parametrize("binding,code", [(None, "plan_guard_unavailable"),
    (SimpleNamespace(active=True), "plan_mode"), (SimpleNamespace(), "plan_guard_unavailable")])
def test_fail_closed_launch_and_all_mutations(binding, code):
    def forbidden(*args):
        pytest.fail("launch happened without permission")
    registered = tools(binding, forbidden)
    async def scenario():
        assert (await call(registered, "KetchupLiveSession", action="launch", executable=sys.executable))["error"]["code"] == code
        for action in ("propose", "commit", "undo", "redo"):
            assert (await call(registered, "KetchupLiveEdit", action=action, handle="bad",
                               expected=STAMP, selection=[]))["error"]["code"] == code
        for action in ("selection", "view"):
            assert (await call(registered, "KetchupLiveView", action=action, handle="bad",
                               expected=STAMP))["error"]["code"] == code
    asyncio.run(scenario())


def test_bound_guard_changes_after_launch_reads_and_disconnect_remain_allowed():
    state = SimpleNamespace(active=False)
    session = SessionDouble()
    registered = tools(state, lambda *args: session)
    async def scenario():
        handle = (await launch(registered))["result"]["handle"]
        state.active = True
        for action in ("status", "summary", "query", "detail"):
            assert (await call(registered, "KetchupLiveInspect", action=action, handle=handle, expected=STAMP))["ok"]
        before = len(session.calls)
        denied = await call(registered, "KetchupLiveEdit", action="undo", handle=handle, expected=STAMP, selection=[])
        assert denied["error"]["code"] == "plan_mode" and len(session.calls) == before
        del state.active
        assert (await call(registered, "KetchupLiveSession", action="disconnect", handle=handle))["ok"]
        assert session.closed
    asyncio.run(scenario())


@pytest.mark.parametrize("key", list(STAMP))
def test_missing_stamp_field_never_calls_mutation(key):
    session = SessionDouble()
    registered = tools(SimpleNamespace(active=False), lambda *args: session)
    incomplete = {k: v for k, v in STAMP.items() if k != key}
    async def scenario():
        handle = (await launch(registered))["result"]["handle"]
        result = await call(registered, "KetchupLiveEdit", action="undo", handle=handle, expected=incomplete, selection=[])
        assert not result["ok"] and not any(c[0] == "undo" for c in session.calls)
    asyncio.run(scenario())


def test_remote_stale_and_unknown_exceptions_are_sanitized_without_retry():
    async def scenario(failure, code, unknown):
        session = SessionDouble()
        registered = tools(SimpleNamespace(active=False), lambda *args: session)
        handle = (await launch(registered))["result"]["handle"]
        session.fail = failure
        result = await call(registered, "KetchupLiveEdit", action="commit", handle=handle,
                            expected=STAMP, selection=[], proposal_id=9)
        assert result["error"]["code"] == code and result["mutation_outcome_unknown"] is unknown
        assert "DO_NOT_EXPOSE" not in json.dumps(result)
        assert result["retry_mutation"] is False
        assert sum(c[0] == "commit" for c in session.calls) == 1
        if unknown:
            assert session.closed
    asyncio.run(scenario(skill._live().LiveBridgeError("stale_document"), "stale_document", False))
    asyncio.run(scenario(RuntimeError("DO_NOT_EXPOSE"), "live_operation_failed", True))
    asyncio.run(scenario(skill._live().LiveTransportError("DO_NOT_EXPOSE", mutation_outcome_unknown=True),
                         "live_transport_error", True))


def test_oversized_result_is_incomplete_and_preserves_stamp():
    result = json.loads(skill._output(envelope({"too_large": "ž" * 32768})))
    assert result["error"]["code"] == "output_too_large"
    assert result["complete"] is False and result["operation_completed"] is True
    assert result["stamp"] == STAMP and result["retry_mutation"] is False


@pytest.mark.parametrize("during_launch", [True, False])
def test_cancel_closes_only_own_session_even_during_launch(during_launch):
    entered, release = threading.Event(), threading.Event()
    session = SessionDouble()
    def blocked(*args):
        entered.set()
        assert release.wait(2)
        return session if during_launch else envelope(stamp=session.stamp)
    registered = tools(SimpleNamespace(active=False), blocked if during_launch else lambda *args: session)
    async def scenario():
        if during_launch:
            task = asyncio.create_task(launch(registered))
        else:
            handle = (await launch(registered))["result"]["handle"]
            session.undo = blocked
            task = asyncio.create_task(call(registered, "KetchupLiveEdit", action="undo", handle=handle,
                                            expected=STAMP, selection=[]))
        assert await asyncio.to_thread(entered.wait, 2)
        task.cancel()
        await asyncio.sleep(0)
        task.cancel()
        release.set()
        with pytest.raises(asyncio.CancelledError):
            await task
        assert session.closed and session.calls[-1] == ("close",)
    asyncio.run(scenario())


class CapturedInput(io.BytesIO):
    def close(self):
        self.captured = self.getvalue()
        super().close()


class ProcessDouble:
    def __init__(self, stdout):
        self.stdin = CapturedInput()
        self.stdout = io.BytesIO(stdout)
    def kill(self):
        pytest.fail("must never kill a GUI")
    def terminate(self):
        pytest.fail("must never terminate a GUI")
    def wait(self, *args, **kwargs):
        pytest.fail("must never wait for GUI exit")


@pytest.fixture
def pipe_double(monkeypatch):
    monkeypatch.setattr(skill, "_pipe_chunk", lambda stream, limit: stream.read(limit))


def test_private_bootstrap_and_nonowning_pipe_cleanup(monkeypatch, pipe_double, tmp_path, capsys):
    token = "a" * 64
    monkeypatch.setattr(skill.secrets, "token_hex", lambda count: token if count == 32 else pytest.fail("entropy size"))
    process = ProcessDouble(b'{"version":1,"live_bridge_address":"127.0.0.1:3456"}\n' + b"discard" * 10000)
    seen = []
    def popen(command, **kwargs):
        seen.append((command, kwargs))
        return process
    monkeypatch.setattr(skill.subprocess, "Popen", popen)
    document = tmp_path / "explicit.ketchup"
    document.write_bytes(b"path validation only")
    session = SessionDouble()
    def factory(address, credential, timeout):
        assert address == "127.0.0.1:3456" and credential == token and 0 < timeout <= 10
        return session
    launched = skill._launch(sys.executable, str(document), session_factory=factory)
    launched.close()
    assert session.closed and process.stdin.closed and process.stdout.closed
    assert json.loads(process.stdin.captured) == {"version": 1, "token": token}
    assert process.stdin.captured.count(b"\n") == 1 and len(process.stdin.captured) <= 1024
    command, kwargs = seen[0]
    assert command == [str(Path(sys.executable).resolve()), "--supervisor-live-stdin", str(document.resolve())]
    assert kwargs == {"stdin": skill.subprocess.PIPE, "stdout": skill.subprocess.PIPE,
                      "stderr": skill.subprocess.DEVNULL, "bufsize": 0, "shell": False}
    assert token not in repr(seen) and token not in repr(launched)
    assert capsys.readouterr() == ("", "")


@pytest.mark.parametrize("stdout", [b"", b"not JSON\n", b"x" * 1025,
    b'{"version":1,"live_bridge_address":"localhost:42"}\n',
    b'{"version":1,"live_bridge_address":"127.0.0.1:0042"}\n',
    b'{"version":true,"live_bridge_address":"127.0.0.1:42"}\n',
    b'{"version":1,"version":1,"live_bridge_address":"127.0.0.1:42"}\n',
    b'{"version":1,"live_bridge_address":"127.0.0.1:42","token":"DO_NOT_EXPOSE"}\n'])
def test_bad_bootstrap_sanitized_no_termination(monkeypatch, pipe_double, stdout):
    process = ProcessDouble(stdout)
    monkeypatch.setattr(skill.subprocess, "Popen", lambda *args, **kwargs: process)
    with pytest.raises(skill.Rejection, match="Live GUI startup failed") as caught:
        skill._launch(sys.executable, session_factory=lambda *args, **kwargs: pytest.fail("must not connect"))
    assert caught.value.__suppress_context__
    assert "DO_NOT_EXPOSE" not in str(caught.value)
    assert process.stdin.closed and process.stdout.closed


def test_startup_deadline_does_not_wait_for_app(monkeypatch):
    process = ProcessDouble(b"")
    monkeypatch.setattr(skill.subprocess, "Popen", lambda *args, **kwargs: process)
    monkeypatch.setattr(skill, "_pipe_chunk", lambda *args: None)
    started = time.monotonic()
    with pytest.raises(skill.Rejection):
        skill._launch(sys.executable, timeout=0.03)
    assert time.monotonic() - started < 1
    assert process.stdout.closed and process.stdin.closed


def test_real_pipe_polling_and_stoppable_drain():
    read_fd, write_fd = os.pipe()
    stream = os.fdopen(read_fd, "rb", buffering=0)
    try:
        assert skill._pipe_chunk(stream, 10) is None
        os.write(write_fd, b"hello")
        assert skill._pipe_chunk(stream, 10) == b"hello"
        drain = skill._Drain(stream)
        drain.close()
        assert not drain.thread.is_alive() and stream.closed
    finally:
        stream.close()
        os.close(write_fd)


def test_invalid_paths_and_no_extra_launch_arguments():
    registered = tools(SimpleNamespace(active=False), lambda *args: pytest.fail("must not launch"))
    async def scenario():
        for executable in ("", "ketchup.exe", str(ROOT / "does-not-exist.exe"), str(ROOT)):
            result = await call(registered, "KetchupLiveSession", action="launch", executable=executable)
            assert result["error"]["code"] == "invalid_path"
    asyncio.run(scenario())


def test_registered_sdk_socket_injection_unknown_commit_no_retry():
    """Actual LiveSession TCP framing, synthetic peer; not Rust/GUI E2E."""
    token = "b" * 64
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    listener.settimeout(2)
    methods, errors = [], []
    def exact(stream, size):
        result = b""
        while len(result) < size:
            data = stream.recv(size - len(result))
            if not data:
                raise EOFError()
            result += data
        return result
    def peer():
        try:
            stream, _ = listener.accept()
            with stream:
                stream.settimeout(2)
                for _ in range(3):
                    size = struct.unpack("!I", exact(stream, 4))[0]
                    request = json.loads(exact(stream, size))
                    assert request["token"] == token
                    method = request["request"]["method"]
                    methods.append(method)
                    if method == "commit":
                        assert request["request"]["expected"] == STAMP
                        return  # Deliberately lose response after receiving mutation.
                    response = envelope({"selection": []})
                    response["id"] = request["id"]
                    data = json.dumps(response).encode()
                    stream.sendall(struct.pack("!I", len(data)) + data)
        except BaseException as error:
            errors.append(error)
    thread = threading.Thread(target=peer, daemon=True)
    thread.start()
    sessions = []
    def factory(*args):
        session = skill._sdk().LiveSession(listener.getsockname(), token, timeout=1)
        sessions.append(session)
        return session
    registered = tools(SimpleNamespace(active=False), factory)
    async def scenario():
        opened = await launch(registered)
        assert opened["stamp"] == STAMP
        result = await call(registered, "KetchupLiveEdit", action="commit", handle=opened["result"]["handle"],
                            expected=STAMP, selection=[], proposal_id=9)
        assert result["mutation_outcome_unknown"] is True and result["retry_mutation"] is False
        assert token not in json.dumps(result)
        assert sessions[0].closed
    try:
        asyncio.run(scenario())
    finally:
        listener.close()
        for session in sessions:
            session.close()
        thread.join(3)
    assert not thread.is_alive() and not errors
    assert methods == ["status", "status", "commit"]


def image_envelope(capture_mode="offscreen"):
    """Synthetic PNG fixture, not runtime visual evidence."""
    visible = capture_mode == "visible_viewport"
    def chunk(kind, data):
        return struct.pack("!I", len(data)) + kind + data + struct.pack("!I", zlib.crc32(kind + data))
    data = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack("!IIBBBBB", 1, 1, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(b"\0\x12\x34\x56\xff")) + chunk(b"IEND", b""))
    return envelope({"data": base64.b64encode(data).decode("ascii"), "width": 1, "height": 1,
        "stamp": copy.deepcopy(STAMP), "capture_stamp": copy.deepcopy(STAMP),
        "mime_type": "image/png", "encoding": "base64", "scope": "cad_viewport",
        "capture_mode": capture_mode, "capture_pass": 17,
        "render": {"render_correlated": True, "callback_correlated": not visible,
                   "viewport_visibility_required": visible, "viewport_unoccluded": visible},
        "capture_id": 13, "render_id": 17, "source": "cad_viewport"})


def test_registered_image_artifact_receipt_no_base64_or_overwrite(tmp_path, monkeypatch):
    monkeypatch.setattr(skill, "IMAGE_ROOT", tmp_path / "artifacts" / "live-view")
    destination = skill.IMAGE_ROOT / "explicit.png"
    session = SessionDouble()
    image_calls = []
    def image(expected, capture_mode="offscreen"):
        image_calls.append((copy.deepcopy(expected), capture_mode))
        return image_envelope()
    session.image = image
    registered = tools(SimpleNamespace(active=False), lambda *args: session)
    async def scenario():
        handle = (await launch(registered))["result"]["handle"]
        result = await call(registered, "KetchupLiveView", action="image", handle=handle,
                            expected=STAMP, image_path=str(destination))
        assert result["ok"] and result["stamp"] == STAMP
        assert result["result"]["capture_stamp"] == STAMP
        assert result["result"]["capture_id"] == 13 and result["result"]["render_id"] == 17
        artifact = result["result"]["artifact"]
        assert artifact["artifact_saved"] and artifact["path"] == str(destination)
        assert artifact["visual_delivery"] == "unverified" and artifact["geometry_evaluated"] is False
        assert "data" not in result["result"]
        assert len(json.dumps(result).encode()) < 2048
        original = destination.read_bytes()
        assert artifact["byte_count"] == len(original) and len(artifact["sha256"]) == 64
        result = await call(registered, "KetchupLiveView", action="image", handle=handle,
                            expected=STAMP, image_path=str(destination))
        assert result["error"]["code"] == "file_exists"
        assert destination.read_bytes() == original and image_calls == [(STAMP, "offscreen")]
    asyncio.run(scenario())


@pytest.mark.parametrize("case", ["missing", "relative", "outside", "extension", "traversal", "selection", "view"])
def test_registered_image_path_scope_preflight(tmp_path, monkeypatch, case):
    monkeypatch.setattr(skill, "IMAGE_ROOT", tmp_path / "live-view")
    session = SessionDouble()
    session.image = lambda *args: pytest.fail("invalid path reached capture")
    registered = tools(SimpleNamespace(active=False), lambda *args: session)
    async def scenario():
        handle = (await launch(registered))["result"]["handle"]
        paths = {"missing": "", "relative": "capture.png", "outside": str(tmp_path / "outside.png"),
                 "extension": str(skill.IMAGE_ROOT / "capture.txt"),
                 "traversal": str(skill.IMAGE_ROOT / ".." / "escape.png")}
        action = case if case in ("selection", "view") else "image"
        result = await call(registered, "KetchupLiveView", action=action, handle=handle, expected=STAMP,
                            image_path=paths.get(case, str(skill.IMAGE_ROOT / "capture.png")))
        assert result["error"]["code"] == ("invalid_arguments" if action != "image" else "invalid_path")
        assert not skill.IMAGE_ROOT.exists()
    asyncio.run(scenario())


@pytest.mark.parametrize("when", ["before", "during", "unbound"])
def test_image_file_write_guard_but_inspection_stays_read_only(tmp_path, monkeypatch, when):
    monkeypatch.setattr(skill, "IMAGE_ROOT", tmp_path)
    destination = tmp_path / "guarded.png"
    state = SimpleNamespace(active=False)
    session = SessionDouble()
    def image(expected, capture_mode="offscreen"):
        assert capture_mode == "offscreen" and when == "during"
        state.active = True
        return image_envelope()
    session.image = image
    registered = tools(state, lambda *args: session)
    async def scenario():
        handle = (await launch(registered))["result"]["handle"]
        if when == "before":
            state.active = True
        elif when == "unbound":
            del state.active
        assert (await call(registered, "KetchupLiveInspect", action="summary", handle=handle))["ok"]
        result = await call(registered, "KetchupLiveView", action="image", handle=handle,
                            expected=STAMP, image_path=str(destination))
        assert result["error"]["code"] == ("plan_guard_unavailable" if when == "unbound" else "plan_mode")
        assert not destination.exists() and not session.closed
    asyncio.run(scenario())


@pytest.mark.parametrize("tamper", ["epoch", "bytes", "capture_stamp"])
def test_registered_image_invalid_response_never_saves_or_leaks(tmp_path, monkeypatch, tamper):
    monkeypatch.setattr(skill, "IMAGE_ROOT", tmp_path)
    destination = tmp_path / "invalid.png"
    session = SessionDouble()
    value = image_envelope()
    if tamper == "epoch":
        value["stamp"]["mutation_epoch"] += 1
    elif tamper == "capture_stamp":
        value["result"]["capture_stamp"]["mutation_epoch"] += 1
    else:
        value["result"]["data"] = "DO_NOT_EXPOSE"
    session.image = lambda expected, capture_mode="offscreen": value
    registered = tools(SimpleNamespace(active=False), lambda *args: session)
    async def scenario():
        handle = (await launch(registered))["result"]["handle"]
        result = await call(registered, "KetchupLiveView", action="image", handle=handle,
                            expected=STAMP, image_path=str(destination))
        assert result["error"]["code"] == "live_transport_error" and not result["mutation_outcome_unknown"]
        assert "DO_NOT_EXPOSE" not in json.dumps(result) and "data" not in json.dumps(result)
        assert not destination.exists() and session.closed
    asyncio.run(scenario())
