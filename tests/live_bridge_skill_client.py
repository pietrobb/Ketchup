"""Private subprocess helper for crates/ketchup-app/tests/live_bridge_python.rs.

Requires Python 3.11+ and anthropic; deliberately not a pytest test module.
Real LiveSession, real registered beta tools and real Rust TCP bridge. The only
injection replaces the host launcher with attachment to the Rust-owned Shell,
plus a shared host plan-state object. It does NOT exercise production launching,
OAuth, desktop input, rendered images, or claim geometry validation.
Credentials arrive once over private stdin, never via tool arguments or output.
"""
from __future__ import annotations

import asyncio
import importlib.util
import json
from pathlib import Path
import sys
from types import SimpleNamespace
from uuid import uuid4

async def scenario():
    # No arbitrary exception text/tracebacks may escape the outer entry point.
    assert sys.version_info >= (3, 11), "Python 3.11+ required"
    attachment = json.loads(sys.stdin.readline(32769))
    assert set(attachment) == {"address", "token", "program"}
    token = attachment.pop("token")
    address = attachment.pop("address")
    program = attachment.pop("program")
    assert isinstance(token, str) and len(token) == 64
    assert isinstance(address, str) and address.startswith("127.0.0.1:")
    root = Path(__file__).resolve().parents[1]
    image_path = root / "artifacts" / "live-view" / f"shell-hidden-{uuid4().hex}.png"
    spec = importlib.util.spec_from_file_location(
        "live_bridge_e2e_skill", root / "skills" / "ketchup_live.py")
    skill = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(skill)  # Imports the actual anthropic beta decorator.
    plan = SimpleNamespace(active=False)
    sessions = []

    def attach(executable, document_path=None):
        # Test-only host seam: not an LLM capability and no subprocess launch.
        assert Path(executable) == Path(sys.executable).resolve()
        assert document_path is None and not sessions
        session = skill._sdk().LiveSession(address, token, timeout=30.0)
        sessions.append(session)
        return session

    skill._plan_state = lambda: plan
    skill._launch = attach
    tools = {tool.name: tool for tool in skill.register_tools()}
    assert set(tools) == {
        "KetchupLiveSession", "KetchupLiveInspect", "KetchupLiveEdit", "KetchupLiveView"}

    def safe(text):
        assert isinstance(text, str) and len(text.encode("utf-8")) <= 32768
        assert token not in text and address not in text, "private attachment leaked"

    for tool in tools.values():
        schema = tool.to_dict()
        safe(json.dumps(schema))
        assert not {"token", "address", "credentials", "launcher", "plan_mode"} & set(
            schema["input_schema"]["properties"])

    async def call(name, **arguments):
        safe(json.dumps(arguments))  # No secret-bearing model-visible arguments.
        output = await tools[name].call(arguments)  # Actual registered beta .call.
        safe(output)
        return json.loads(output)

    def success(response):
        assert response.get("ok") is True, "registered tool rejected operation"
        return response

    def rejected(response, code):
        assert response.get("ok") is False
        assert response["error"]["code"] in ((code,) if isinstance(code, str) else code)
        assert response.get("mutation_outcome_unknown") is False
        assert response.get("retry_mutation") is False

    def checkpoint(name, stamp):
        # Only static checkpoint labels and nonsecret validated stamps escape.
        text = json.dumps({"checkpoint": name, "stamp": stamp}, separators=(",", ":"))
        safe(text)
        print(text, flush=True)
        assert sys.stdin.readline(32) == "continue\n", "Rust checkpoint acknowledgement missing"

    launched = success(await call(
        "KetchupLiveSession", action="launch", executable=str(Path(sys.executable).resolve())))
    handle = launched["result"]["handle"]
    assert len(sessions) == 1
    assert type(sessions[0]) is skill._sdk().LiveSession  # Never a fake SDK session.

    async def inspect(action="status", **arguments):
        return success(await call("KetchupLiveInspect", action=action, handle=handle, **arguments))

    async def edit(action, expected, selection, **arguments):
        return await call("KetchupLiveEdit", action=action, handle=handle,
                          expected=expected, selection=selection, **arguments)

    state = await inspect()
    initial = state["stamp"]
    selection = state["result"]["selection"]
    assert isinstance(selection, list)
    summary = await inspect("summary")
    assert summary["stamp"] == initial == launched["stamp"]
    checkpoint("initial", initial)

    plan.active = True
    for action, arguments in (
        ("propose", {"program": program}), ("commit", {"proposal_id": 1}),
        ("undo", {}), ("redo", {}),
    ):
        rejected(await edit(action, initial, selection, **arguments), "plan_mode")
    for action, arguments in (
        ("selection", {"occurrence_ids": []}), ("view", {"view": "top"}),
        ("image", {"image_path": str(image_path)})):
        rejected(await call("KetchupLiveView", action=action, handle=handle,
                            expected=initial, **arguments), "plan_mode")
    rejected(await call("KetchupLiveSession", action="launch",
                        executable=str(Path(sys.executable).resolve())), "plan_mode")
    assert len(sessions) == 1
    assert (await inspect("summary"))["stamp"] == initial  # Reads remain allowed.
    guarded = await inspect()
    assert guarded["result"]["selection"] == selection
    assert guarded["result"]["pending_proposal_id"] is None
    checkpoint("plan_guarded", guarded["stamp"])
    plan.active = False

    proposed = success(await edit("propose", initial, selection, program=program))
    assert proposed["stamp"] == initial
    proposal_id = proposed["result"]["proposal_id"]
    checkpoint("proposed", proposed["stamp"])
    committed = success(await edit("commit", initial, selection, proposal_id=proposal_id))
    checkpoint("committed", committed["stamp"])

    state = await inspect()
    before_aba = state["stamp"]
    selection = state["result"]["selection"]
    pending = success(await edit("propose", before_aba, selection, program=program))
    pending_id = pending["result"]["proposal_id"]
    checkpoint("aba_ready", pending["stamp"])
    # Rust has now invoked actual AccessKit Undo then Redo, restoring revision
    # and digest but advancing epoch. Do not refresh the caller's stale guard.
    rejected(await edit("commit", before_aba, selection, proposal_id=pending_id), "stale_document")
    # The skill preflight itself rejects stale stamps using real socket status.
    # Also prove Rust's independent atomic guard via the SAME real SDK socket,
    # without bypassing/replacing any SDK implementation or mutating authority.
    try:
        sessions[0].commit(before_aba, pending_id)
    except skill._live().LiveBridgeError as error:
        assert error.code == "stale_document"
    else:
        raise AssertionError("Rust accepted stale proposal")
    state = await inspect()
    assert state["stamp"]["revision"] == before_aba["revision"]
    assert state["stamp"]["canonical_digest"] == before_aba["canonical_digest"]
    assert state["stamp"]["mutation_epoch"] > before_aba["mutation_epoch"]
    checkpoint("stale_rejected", state["stamp"])

    undone = success(await edit("undo", state["stamp"], state["result"]["selection"]))
    checkpoint("undone", undone["stamp"])
    state = await inspect()
    redone = success(await edit("redo", state["stamp"], state["result"]["selection"]))
    checkpoint("redone", redone["stamp"])

    rejected(await call("KetchupLiveView", action="image", handle=handle,
                        expected=redone["stamp"], image_path=str(image_path)), ("image_timeout", "stale_image"))  # Exact publication can invalidate before timeout.
    state = await inspect()
    assert state["result"]["image"] == "cad_viewport_png_thumbnail"
    assert state["stamp"] == redone["stamp"] and not image_path.exists()
    checkpoint("image_renderer_unavailable", state["stamp"])

    plan.active = True  # Disconnect is cleanup, not ownership of the GUI.
    disconnected = success(await call("KetchupLiveSession", action="disconnect", handle=handle))
    assert disconnected["result"]["app_terminated"] is False
    assert sessions[0].closed
    rejected(await call("KetchupLiveInspect", action="status", handle=handle), "invalid_handle")
    checkpoint("disconnected", state["stamp"])


if __name__ == "__main__":
    try:
        asyncio.run(scenario())
    except BaseException:
        # Do not print exception repr/traceback, locals, tool returns or input.
        # Rust reports the expected stage, not any child-provided diagnostics.
        print('{"checkpoint":"failed"}', flush=True)
        sys.exit(1)
