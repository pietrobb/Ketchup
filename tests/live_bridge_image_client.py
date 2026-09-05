"""Private rendered-image E2E helper; not a pytest module or native launcher.

Separate from the mutation/history Shell helper because this host actually services
isolated CAD wgpu callbacks. Credentials arrive only over private stdin. Only
validated nonsecret checkpoints/receipts leave stdout; exceptions are suppressed.
"""
from __future__ import annotations

import asyncio
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
from types import SimpleNamespace


async def scenario():
    assert sys.version_info >= (3, 11)
    attachment = json.loads(sys.stdin.readline(32769))
    assert set(attachment) == {"address", "token", "image_path"}
    token = attachment.pop("token")
    address = attachment.pop("address")
    destination = Path(attachment.pop("image_path"))
    assert isinstance(token, str) and len(token) == 64
    assert isinstance(address, str) and address.startswith("127.0.0.1:")
    root = Path(__file__).resolve().parents[1]
    assert destination.is_absolute() and destination.suffix == ".png"
    destination.relative_to(root / "artifacts" / "live-view")
    assert not destination.exists()
    spec = importlib.util.spec_from_file_location(
        "live_bridge_image_e2e_skill", root / "skills" / "ketchup_live.py")
    skill = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(skill)  # Actual installed anthropic beta decorator.
    plan = SimpleNamespace(active=False)
    sessions = []

    def attach(executable, document_path=None):
        assert Path(executable) == Path(sys.executable).resolve()
        assert document_path is None and not sessions
        session = skill._sdk().LiveSession(address, token, timeout=30.0)
        sessions.append(session)
        return session

    skill._launch = attach  # Trusted test host attachment, not production launch.
    skill._plan_state = lambda: plan
    tools = {tool.name: tool for tool in skill.register_tools()}

    def safe(text):
        assert isinstance(text, str) and len(text.encode("utf-8")) <= 32768
        assert token not in text and address not in text

    for tool in tools.values():
        schema = tool.to_dict()
        safe(json.dumps(schema))
        assert not {"token", "address", "credentials", "launcher", "plan_mode"} & set(
            schema["input_schema"]["properties"])
    assert "image_path" in tools["KetchupLiveView"].to_dict()["input_schema"]["properties"]

    async def call(name, **arguments):
        safe(json.dumps(arguments))
        text = await tools[name].call(arguments)
        safe(text)
        return json.loads(text)

    def success(response):
        assert response.get("ok") is True
        return response

    def rejected(response, code):
        assert response.get("ok") is False and response["error"]["code"] == code
        assert response.get("mutation_outcome_unknown") is False
        assert response.get("retry_mutation") is False

    def checkpoint(name, stamp, receipt=None):
        event = {"checkpoint": name, "stamp": stamp}
        if receipt is not None:
            event["receipt"] = receipt
        text = json.dumps(event, separators=(",", ":"))
        safe(text)
        print(text, flush=True)
        assert sys.stdin.readline(32) == "continue\n"

    launched = success(await call("KetchupLiveSession", action="launch",
                                  executable=str(Path(sys.executable).resolve())))
    handle = launched["result"]["handle"]
    assert len(sessions) == 1 and type(sessions[0]) is skill._sdk().LiveSession
    state = success(await call("KetchupLiveInspect", action="status", handle=handle))
    stamp = state["stamp"]
    assert launched["stamp"] == stamp
    assert state["result"]["image"] == "cad_viewport_png_thumbnail"
    checkpoint("initial", stamp)

    async def image(expected=stamp, image_path=str(destination)):
        return await call("KetchupLiveView", action="image", handle=handle,
                          expected=expected, image_path=image_path)

    plan.active = True
    rejected(await image(), "plan_mode")
    assert not destination.exists()
    assert success(await call("KetchupLiveInspect", action="summary", handle=handle))["stamp"] == stamp
    checkpoint("plan_guarded", stamp)
    plan.active = False
    rejected(await image(image_path=""), "invalid_path")
    stale = dict(stamp, mutation_epoch=stamp["mutation_epoch"] + 1)
    rejected(await image(expected=stale), "stale_document")
    assert not destination.exists()
    receipt = success(await image())  # Rust renderer -> TCP -> real SDK -> beta .call.
    result = receipt["result"]
    assert receipt["stamp"] == result["stamp"] == stamp
    assert result["render"]["source"] == "isolated_cad_target"
    assert result["render"]["gui_overlays_included"] is False
    assert "data" not in result and "png_base64" not in result
    assert result["scope"] == "cad_viewport" and result["mime_type"] == "image/png"
    assert result["render"]["callback_correlated"] is True
    assert result["render"]["viewport_unoccluded"] is True
    assert result["render"]["geometry_complete"] is False
    assert result["render"]["completeness"] == "display_only_not_geometry_validation"
    artifact = result["artifact"]
    assert Path(artifact["path"]) == destination
    assert artifact["artifact_saved"] is True
    assert artifact["visual_delivery"] == "unverified"
    assert artifact["geometry_evaluated"] is False
    png = destination.read_bytes()  # Missing artifact MUST fail, never count as success.
    assert png.startswith(b"\x89PNG\r\n\x1a\n")
    assert artifact["byte_count"] == len(png) > 57
    assert artifact["sha256"] == hashlib.sha256(png).hexdigest()
    checkpoint("captured", stamp, receipt)

    rejected(await image(), "file_exists")
    assert destination.read_bytes() == png
    state = success(await call("KetchupLiveInspect", action="status", handle=handle))
    assert state["stamp"] == stamp
    checkpoint("create_only", stamp)
    plan.active = True
    closed = success(await call("KetchupLiveSession", action="disconnect", handle=handle))
    assert closed["result"]["app_terminated"] is False and sessions[0].closed
    checkpoint("disconnected", stamp)


if __name__ == "__main__":
    try:
        asyncio.run(scenario())
    except BaseException:
        print('{"checkpoint":"failed"}', flush=True)
        sys.exit(1)
