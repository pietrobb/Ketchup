"""Contract/safety doubles are NOT geometry evidence; the last test uses real tools/SDK."""
import asyncio
import copy
import importlib.util
import json
import os
from pathlib import Path
import sys
import threading
from types import SimpleNamespace
import uuid

import pytest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("tested_ketchup_model", ROOT / "skills/ketchup_model.py")
skill = importlib.util.module_from_spec(spec)
spec.loader.exec_module(skill)


def test_output_is_bounded_real_json():
    for value in ("small", "ž" * 20000, "x" * 100000):
        result = skill._output({"ok": True, "result": {"report": value}})
        assert len(result.encode("utf-8")) <= 32768
        parsed = json.loads(result)
        if len(value) > 100:
            assert parsed["error"]["code"] == "output_too_large"
            assert parsed["complete"] is False
            assert parsed["operation_completed"] is True
    with pytest.raises(ValueError):
        skill._output({"result": float("nan")})


def test_guard_fails_closed_and_tracks_shared_state():
    with pytest.raises(skill.Rejection, match="No Supervisor"):
        skill.Runtime(None).guard()
    state = SimpleNamespace(active=False)
    runtime = skill.Runtime(state)
    runtime.guard()
    state.active = True
    with pytest.raises(skill.Rejection, match="plan mode"):
        runtime.guard()


def test_invalid_handles_and_paths():
    runtime = skill.Runtime(None)
    for handle in ("", "not-a-uuid", "00000000-0000-0000-0000-000000000000"):
        with pytest.raises(skill.Rejection, match="session UUID"):
            runtime.entry(handle)
    for path in ("", "relative.ketchup"):
        with pytest.raises(skill.Rejection, match="absolute"):
            skill._absolute(path)


def test_sdk_load_does_not_shadow_global_package():
    sentinel = object()
    original = sys.modules.get("ketchup", sentinel)
    before = list(sys.path)
    assert skill._sdk().Session
    assert sys.modules.get("ketchup", sentinel) is original
    assert sys.path == before


def test_registration_time_supervisor_plan_binding():
    namespace = {"__name__": "src.claude_engine", "skill": skill}
    exec("class ClaudeEngine:\n def load(self):\n  return skill._plan_state()", namespace)
    engine = namespace["ClaudeEngine"]()
    engine._plan_state = SimpleNamespace(active=False)
    assert engine.load() is engine._plan_state
    assert skill._plan_state() is None


def tools(monkeypatch, state=None):
    monkeypatch.setattr(skill, "_plan_state", lambda: state or SimpleNamespace(active=False))
    return {tool.name: tool for tool in skill.register_tools()}


async def call(registered, name, **kwargs):
    text = await registered[name].call(kwargs)
    assert isinstance(text, str)
    assert len(text.encode("utf-8")) <= 32768
    return json.loads(text)


def test_registration_schema_and_real_decorator_calls(monkeypatch):
    registered = tools(monkeypatch)
    assert set(registered) == {"KetchupDiscover", "KetchupSession", "KetchupInspect", "KetchupEdit", "KetchupSave", "KetchupVerify"}
    for tool in registered.values():
        schema = tool.to_dict()
        assert schema["input_schema"]["type"] == "object"
        assert "plan_mode" not in schema["input_schema"]["properties"]
        json.dumps(schema)
    required = registered["KetchupEdit"].to_dict()["input_schema"]["required"]
    assert {"expected_revision", "expected_digest"} <= set(required)
    async def scenario():
        assert (await call(registered, "KetchupDiscover"))["result"]["backend_compact"]
        for name in ("KetchupSession", "KetchupInspect", "KetchupEdit", "KetchupVerify"):
            args = {"action": "invalid", "handle": "bad"}
            if name == "KetchupEdit":
                args.update(expected_revision=0, expected_digest="x")
            assert (await call(registered, name, **args))["error"]["code"] == "invalid_action"
        assert (await call(registered, "KetchupInspect", handle="bad"))["error"]["code"] == "invalid_handle"
    asyncio.run(scenario())


def test_preconditions_check_observed_and_fresh_identity():
    identity = {"document_id": 1, "revision": 2, "canonical_digest": "abc"}
    entry = {"document_id": 1, "observed": identity}
    skill._precondition(entry, identity, 2, "abc")
    for fresh, rev, digest in ((identity, -1, ""), ({**identity, "revision": 3}, 2, "abc"),
                               ({**identity, "document_id": 4}, 2, "abc"), (identity, 2, "old")):
        with pytest.raises(skill.Rejection):
            skill._precondition(entry, fresh, rev, digest)
    entry["observed"] = None
    with pytest.raises(skill.Rejection):
        skill._precondition(entry, identity, 2, "abc")


class SafetyDocument:
    def __init__(self):
        self.snapshot = {"document_id": str(uuid.uuid4()), "revision": 0, "canonical_digest": "zero",
                         "undo_steps": 0, "redo_steps": 0}
        self.calls = []
        self.validators = SimpleNamespace(list=lambda: {"validators": []}, run=lambda ids: {"ids": ids})

    @property
    def state(self):
        raise AssertionError("Skill must never request legacy state")

    def summary(self):
        return {"state": copy.deepcopy(self.snapshot), "summary": {
            "identity": skill._identity(self.snapshot), "counts": {"root_occurrences": 0, "definitions": 0, "features": 0},
            "complete": True, "coverage": {"spatial": False}}}

    def query(self, **params):
        self.calls.append(("query", params))
        return {"identity": skill._identity(self.snapshot), "items": [], "next_cursor": None, "complete": True}

    def detail(self, kind, id):
        self.calls.append(("detail", kind, id))
        return {"identity": skill._identity(self.snapshot), "item": {"id": id}, "completeness": {"metadata_only": True}}

    def apply(self, program, *, selection):
        self.calls.append(("apply", program, selection))
        self.snapshot.update(revision=1, canonical_digest="one", undo_steps=1)
        return {**self.summary(), "created": {}}

    def save(self, path, *, overwrite):
        self.calls.append(("save", path, overwrite))
        return self.summary()

    def evaluate(self, **kwargs):
        return {"complete": True, "rows": ["ž" * 40000]}


@pytest.fixture
def doubles(monkeypatch):
    instances = []
    class Session:
        def __init__(self, **kwargs):
            assert kwargs["compact"] is True
            self.doc, self.closed = SafetyDocument(), False
            instances.append(self)
        def new_document(self):
            return self.doc
        def open_document(self, path):
            return self.doc
        def close(self):
            self.closed = True
    monkeypatch.setattr(skill._sdk(), "Session", Session)
    monkeypatch.setenv("KETCHUP_HEADLESS", str(ROOT / "explicit-test-double.exe"))
    return instances


def expected(result):
    identity = result["identity"]
    return {"expected_revision": identity["revision"], "expected_digest": identity["canonical_digest"]}


def test_tool_lifecycle_stale_save_close_and_bounds(monkeypatch, doubles, tmp_path):
    registered = tools(monkeypatch)
    async def scenario():
        opened = (await call(registered, "KetchupSession", action="new"))["result"]
        handle = opened["handle"]
        assert str(uuid.UUID(handle)) == handle
        pre = expected(opened)
        rejected = await call(registered, "KetchupSession", action="close", handle=handle, **pre)
        assert rejected["error"]["code"] == "unsaved_changes"
        assert not doubles[0].closed
        changed = await call(registered, "KetchupEdit", handle=handle, action="apply", program={"operations": []}, **pre)
        assert changed["ok"]
        stale = await call(registered, "KetchupEdit", handle=handle, action="apply", program={"operations": []}, **pre)
        assert stale["error"]["code"] == "stale_precondition"
        assert len(doubles[0].doc.calls) == 1
        current = (await call(registered, "KetchupInspect", handle=handle))["result"]
        pre = expected(current)
        destination = tmp_path / "existing.ketchup"
        destination.write_bytes(b"original")
        assert (await call(registered, "KetchupSave", handle=handle, path=str(destination), **pre))["error"]["code"] == "file_exists"
        assert destination.read_bytes() == b"original"
        assert (await call(registered, "KetchupSave", handle=handle, path=str(destination), overwrite=True, **pre))["ok"]
        assert doubles[0].doc.calls[-1][-1] is True
        report = await call(registered, "KetchupVerify", handle=handle)
        assert report["complete"] is False and report["error"]["code"] == "output_too_large"
        current = (await call(registered, "KetchupInspect", handle=handle))["result"]
        assert (await call(registered, "KetchupSession", action="close", handle=handle, **expected(current)))["ok"]
        assert doubles[0].closed
    asyncio.run(scenario())


def test_compact_queries_and_sectioned_capabilities(monkeypatch, doubles):
    registered = tools(monkeypatch)
    async def scenario():
        opened = (await call(registered, "KetchupSession", action="new"))["result"]
        handle = opened["handle"]
        assert (await call(registered, "KetchupInspect", handle=handle, action="search", cursor="opaque", definition_id=3))["ok"]
        assert doubles[0].doc.calls[-1][1] == {"kind": "occurrences", "search": "", "limit": 20, "cursor": "opaque", "definition_id": 3}
        assert (await call(registered, "KetchupInspect", handle=handle, action="detail", entity_id=4))["result"]["item"]["id"] == 4
        assert (await call(registered, "KetchupInspect", handle=handle, limit=101))["error"]["code"] == "invalid_query"
        assert (await call(registered, "KetchupSession", action="close", handle=handle, discard=True, **expected(opened)))["ok"]
    asyncio.run(scenario())
    variant = {"properties": {"operation": {"const": "transform"}}}
    caps = {"methods": [], "cad_program_schema": {"$defs": {"AssistantCadEditOperation": {"oneOf": [variant]}, "Other": {"type": "string"}}}}
    assert "cad_program_schema" not in skill._capability(caps, "methods", "")
    assert skill._capability(caps, "operations", "")["operations"] == ["transform"]
    assert skill._capability(caps, "operation", "transform")["schema"] == variant
    assert skill._capability(caps, "definition", "Other")["schema"] == {"type": "string"}
    with pytest.raises(skill.Rejection):
        skill._capability(caps, "operation", "invented")


def test_session_limit_and_explicit_executable(monkeypatch, doubles):
    registered = tools(monkeypatch)
    async def scenario():
        monkeypatch.delenv("KETCHUP_HEADLESS")
        assert (await call(registered, "KetchupSession", action="new"))["error"]["code"] == "executable_required"
        monkeypatch.setenv("KETCHUP_HEADLESS", "explicit-env")
        handles = [(await call(registered, "KetchupSession", action="new"))["result"] for _ in range(4)]
        assert len({h["handle"] for h in handles}) == 4
        assert (await call(registered, "KetchupSession", action="new"))["error"]["code"] == "session_limit"
        for h in handles:
            assert (await call(registered, "KetchupSession", action="close", handle=h["handle"], discard=True, **expected(h)))["ok"]
    asyncio.run(scenario())


def test_plan_tools_reject_and_missing_binding(monkeypatch, doubles):
    state = SimpleNamespace(active=False)
    registered = tools(monkeypatch, state)
    async def scenario():
        opened = (await call(registered, "KetchupSession", action="new"))["result"]
        state.active = True
        assert (await call(registered, "KetchupInspect", handle=opened["handle"]))["ok"]
        for name, args in (("KetchupEdit", {"action": "undo"}), ("KetchupSave", {"path": str(ROOT / "never.ketchup")}),
                           ("KetchupSession", {"action": "close", "discard": True})):
            result = await call(registered, name, handle=opened["handle"], **expected(opened), **args)
            assert result["error"]["code"] == "plan_mode"
        state.active = False
        await call(registered, "KetchupSession", action="close", handle=opened["handle"], discard=True, **expected(opened))
        monkeypatch.setattr(skill, "_plan_state", lambda: None)
        unbound = {t.name: t for t in skill.register_tools()}
        assert (await call(unbound, "KetchupSession", action="new"))["error"]["code"] == "plan_guard_unavailable"
    asyncio.run(scenario())


def test_errors_transport_and_cancellation_close_only_owned():
    async def scenario():
        runtime = skill.Runtime(None)
        handle = str(uuid.uuid4())
        closed = []
        runtime.sessions[handle] = {"session": SimpleNamespace(close=lambda: closed.append(handle))}
        def rejection():
            raise skill._sdk().HeadlessError("typed_rejection", "no", {"reason": 1})
        result = json.loads(await runtime.run(handle, rejection))
        assert result["error"]["details"] == {"reason": 1}
        assert not closed
        def transport():
            raise skill._sdk().TransportTimeout("unknown outcome")
        result = json.loads(await runtime.run(handle, transport))
        assert result["retry_mutation"] is False
        assert closed == [handle] and handle not in runtime.sessions
        runtime.sessions[handle] = {"session": SimpleNamespace(close=lambda: closed.append(handle))}
        started, finish = threading.Event(), threading.Event()
        def work():
            started.set()
            finish.wait(2)
            return {"done": True}
        task = asyncio.create_task(runtime.run(handle, work))
        await asyncio.to_thread(started.wait, 2); task.cancel()
        await asyncio.sleep(0); task.cancel()
        await asyncio.sleep(0); assert not task.done() and runtime.lock.locked(); finish.set()
        with pytest.raises(asyncio.CancelledError):
            await task
        assert closed == [handle, handle] and not runtime.sessions
    asyncio.run(scenario())


@pytest.mark.skipif(not os.environ.get("KETCHUP_HEADLESS"), reason="Set KETCHUP_HEADLESS to updated real build; optional KETCHUP_EXACT_WORKER")
def test_real_tool_call_create_query_edit_undo_save_open(monkeypatch, tmp_path):
    registered = tools(monkeypatch)
    async def scenario():
        live = {}
        async def checked(name, **args):
            result = await call(registered, name, **args)
            assert result["ok"], result
            return result["result"]
        async def observe(handle):
            result = await checked("KetchupInspect", handle=handle)
            live[handle] = result
            return result
        try:
            first = await checked("KetchupSession", action="new")
            handle = first["handle"]
            live[handle] = first
            assert first["backend_compact"]
            program = {"operations": [{"operation": "create_part", "name": "Tool evidence",
                       "workplane": {"type": "principal", "plane": "xy"},
                       "entities": skill._sdk().rectangle(20, 10), "constraints": [],
                       "feature": {"type": "extrusion", "distance_mm": 5}, "translation_mm": [0, 0, 0]}]}
            await checked("KetchupEdit", handle=handle, action="apply", program=program, **expected(first))
            base = await observe(handle)
            query = await checked("KetchupInspect", handle=handle, action="search", search="Tool evidence")
            occurrence = query["items"][0]["id"]
            detail = await checked("KetchupInspect", handle=handle, action="detail", entity_id=occurrence)
            assert detail["item"]["id"] == occurrence
            move = {"operations": [{"operation": "transform", "selector": {"type": "occurrences", "occurrence_ids": [occurrence]}, "translation_mm": [3, 0, 0]}]}
            await checked("KetchupEdit", handle=handle, action="apply", program=move, **expected(base))
            moved = await observe(handle)
            assert moved["identity"]["canonical_digest"] != base["identity"]["canonical_digest"]
            await checked("KetchupEdit", handle=handle, action="undo", **expected(moved))
            undone = await observe(handle)
            assert undone["identity"]["canonical_digest"] == base["identity"]["canonical_digest"]
            await checked("KetchupEdit", handle=handle, action="redo", **expected(undone))
            redone = await observe(handle)
            assert redone["identity"]["canonical_digest"] == moved["identity"]["canonical_digest"]
            evaluation = await checked("KetchupVerify", handle=handle)
            assert evaluation["complete"] is True
            path = str(tmp_path / "skill-evidence.ketchup")
            saved = await checked("KetchupSave", handle=handle, path=path, **expected(redone))
            await observe(handle)
            reopened = await checked("KetchupSession", action="open", path=path)
            live[reopened["handle"]] = reopened
            assert reopened["identity"]["canonical_digest"] == saved["identity"]["canonical_digest"]
            assert reopened["counts"] == saved["counts"]
        finally:
            for handle in live:
                observed = await observe(handle)
                await checked("KetchupSession", action="close", handle=handle, discard=True, **expected(observed))
    asyncio.run(scenario())
