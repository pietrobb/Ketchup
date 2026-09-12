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
        "KetchupLiveSession", "KetchupLiveInspect", "KetchupLiveEdit", "KetchupLiveFile",
        "KetchupLiveBatch", "KetchupLiveView"}

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
        assert response.get("ok") is True, response.get("error", {}).get("code")
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
    topology_program = json.loads(json.dumps(program))
    topology_items = {}
    for kind in ("faces", "edges"):
        page = await inspect("query", expected=initial, kind=kind, limit=1)
        assert page["stamp"] == initial
        assert page["result"]["coverage"]["topology"] is True
        assert page["result"]["coverage"]["geometry_evaluated"] is True
        assert page["result"]["items"]
        item = page["result"]["items"][0]
        detail = await inspect("detail", expected=initial, kind=kind, entity_id=item["id"])
        assert detail["stamp"] == initial
        assert detail["result"]["item"] == item
        assert detail["result"]["identity"] == page["result"]["identity"]
        topology_items[kind] = item
    edge = topology_items["edges"]
    topology_program["operations"].append({
        "operation": "append_feature",
        "definition_id": edge["definition_id"],
        "name": "Live topology fillet",
        "feature": {
            "type": "topology_fillet",
            "target_feature_id": edge["producer_feature_id"],
            "edge_reference_ids": [edge["reference_id"]],
            "radius_mm": 1.0,
        },
    })
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

    proposed = success(await edit("propose", initial, selection, program=topology_program))
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


async def consent_disconnect_scenario():
    attachment = json.loads(sys.stdin.readline(32769))
    root = Path(__file__).resolve().parents[1]
    discovery_root = Path(attachment["discovery_root"])
    instance_id = attachment["instance_id"]
    spec = importlib.util.spec_from_file_location(
        "live_disconnect_skill", root / "skills" / "ketchup_live.py")
    skill = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(skill)
    sdk = skill._live()
    plan = SimpleNamespace(active=False)
    tools = {tool.name: tool for tool in skill._register_tools(
        plan,
        discoverer=lambda: sdk._list_live_instances(discovery_root, timeout=2),
        attacher=lambda selected: sdk._attach_live_instance(
            selected, discovery_root, discovery_timeout=2),
    )}

    async def call(name, **arguments):
        text = await tools[name].call(arguments)
        assert len(text.encode("utf-8")) <= 32768
        assert "token" not in text and "address" not in text
        result = json.loads(text)
        if name == "KetchupLiveSession":
            assert result["ok"], result.get("error", {}).get("code")
        return result

    def checkpoint(name, stamp):
        print(json.dumps({"checkpoint": name, "stamp": stamp}), flush=True)
        assert sys.stdin.readline(32) == "continue\n"

    initial = None
    handles = set()
    for attempt in range(2):
        listed = await call("KetchupLiveSession", action="list")
        assert listed["ok"], listed.get("error", {}).get("code")
        assert listed["result"]["instances"] == [{
            "instance_id": instance_id, "document": "Untitled", "status": "available"}], "discovery_available"
        plan.active = False
        attached = await call("KetchupLiveSession", action="attach", instance_id=instance_id)
        assert attached["ok"], attached.get("error", {}).get("code")
        handle = attached["result"]["handle"]
        assert handle not in handles
        handles.add(handle)
        initial = initial or attached["stamp"]
        assert attached["stamp"] == initial
        checkpoint(f"attached_{attempt}", initial)
        busy = await call("KetchupLiveSession", action="list")
        assert busy["result"]["instances"][0]["status"] == "busy", "discovery_busy"
        plan.active = True
        result = await call("KetchupLiveSession", action="disconnect", handle=handle)
        assert result["ok"] and result["result"]["app_terminated"] is False
        invalid = await call("KetchupLiveInspect", action="status", handle=handle)
        assert invalid["error"]["code"] == "invalid_handle"
        checkpoint(f"disconnected_{attempt}", initial)
    listed = await call("KetchupLiveSession", action="list")
    assert listed["result"]["instances"][0]["status"] == "available"
    checkpoint("finished", initial)


if __name__ == "__main__":
    try:
        asyncio.run(consent_disconnect_scenario() if sys.argv[1:] == ["consent-disconnect"] else scenario())
    except BaseException as error:
        # Only helper line numbers escape; never exception text, locals or input.
        lines = []
        trace = error.__traceback__
        while trace is not None:
            if trace.tb_frame.f_code.co_filename == __file__:
                lines.append(trace.tb_lineno)
            trace = trace.tb_next
        code = str(error) if str(error) in {"live_operation_failed", "live_transport_error", "invalid_arguments", "busy", "stale_document", "invalid_program", "resource_limit", "planning_rejected", "invalid_handle", "plan_mode", "output_too_large", "instance_unavailable", "consent_rejected", "discovery_available", "discovery_busy"} else "suppressed"
        print(json.dumps({"checkpoint": "failed", "lines": [*lines, code, type(error).__name__]}), flush=True)
        sys.exit(1)
