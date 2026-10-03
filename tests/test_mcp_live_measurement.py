"""Opt-in public MCP acceptance on a newly launched normal GUI, never an existing window.

Set KETCHUP_APP to a fresh normal build with its matching exact worker alongside.
Only processes descended from this test's MCP server are stopped during cleanup.
"""
import json
import os
from pathlib import Path
import queue
import subprocess
import threading
import time

import pytest


@pytest.fixture
def mcp(tmp_path):
    configured = os.environ.get("KETCHUP_APP")
    if not configured:
        pytest.skip("set KETCHUP_APP to a fresh normal GUI build")
    import psutil

    executable = Path(configured).resolve(strict=True)
    env = os.environ.copy()
    env["APPDATA"] = str(tmp_path / "roaming")
    env["LOCALAPPDATA"] = str(tmp_path / "local")
    Path(env["APPDATA"]).mkdir()
    Path(env["LOCALAPPDATA"]).mkdir()
    lines = queue.Queue()
    metrics = []
    with (tmp_path / "mcp.stderr.log").open("w", encoding="utf-8") as errors:
        server = subprocess.Popen(
            [str(executable), "--mcp"], cwd=tmp_path, env=env,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors,
            text=True, encoding="utf-8",
        )
        def receive():
            for line in server.stdout:
                lines.put(line)
            lines.put(None)
        reader = threading.Thread(target=receive, daemon=True)
        reader.start()
        def request(method, params):
            message = {"jsonrpc": "2.0", "id": len(metrics) + 1, "method": method, "params": params}
            encoded = json.dumps(message)
            started = time.monotonic()
            server.stdin.write(encoded + "\n")
            server.stdin.flush()
            line = lines.get(timeout=90)
            assert line is not None, (tmp_path / "mcp.stderr.log").read_text(encoding="utf-8")
            reply = json.loads(line)
            assert reply["id"] == message["id"], reply
            assert "error" not in reply, reply
            metrics.append({"method": method, "tool": params.get("name"), "action": params.get("arguments", {}).get("action"), "request_bytes": len(encoded.encode()), "response_bytes": len(line.encode()), "seconds": time.monotonic() - started})
            return reply["result"]
        def call(tool_name, error=False, **arguments):
            reply = request("tools/call", {"name": tool_name, "arguments": arguments})
            payload = json.loads(reply["content"][0]["text"])
            if tool_name == "program" and arguments.get("action") in {"apply", "patch", "validate"}:
                (tmp_path / "last-program-result.json").write_text(json.dumps(payload, indent=2), encoding="utf-8")
            if payload.get("error") == "connection_lost":
                status = request("tools/call", {"name": "inspect", "arguments": {"action": "status"}})
                (tmp_path / "failed-request-status.json").write_text(json.dumps(status, indent=2), encoding="utf-8")
            assert reply.get("isError", False) is error, payload
            return payload
        try:
            request("initialize", {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "measurement-acceptance", "version": "1"}})
            call("open_window")
            call.metrics = metrics
            yield call
        finally:
            (tmp_path / "mcp.metrics.json").write_text(json.dumps(metrics, indent=2), encoding="utf-8")
            parent = psutil.Process(server.pid)
            children = parent.children(recursive=True)
            for child in reversed(children):
                try:
                    child.terminate()
                except psutil.NoSuchProcess:
                    pass
            server.terminate()
            server.wait(timeout=15)
            psutil.wait_procs(children, timeout=15)
            server.stdin.close()
            server.stdout.close()


def test_native_face_measurement_and_validation_are_read_only_through_public_mcp(mcp, tmp_path):
    source = "bottom=box('bottom',(100,100,18))\nhole(bottom,'z+',at=(50,50),diameter=8,depth=12)\nshelf=box('shelf',(100,100,18),at=(0,0,668))\nexpect_gap(bottom,shelf,650)\n"
    applied = mcp("program", action="apply", file_name="measurement.star", source=source)
    assert applied["result"]["undo_steps"] == 1
    assert applied["result"]["exact_collisions"]["measurements"][0]["distance_mm"] == pytest.approx(650)
    before = mcp("inspect", action="status")
    stamp = before["stamp"]
    occurrences = mcp("inspect", action="query", kind="occurrences", limit=100)["result"]["items"]
    deadline = time.monotonic() + 30
    while True:
        page = mcp("inspect", action="query", kind="faces", limit=100)["result"]
        faces = list(page["items"])
        while page["next_cursor"] is not None:
            page = mcp("inspect", action="query", kind="faces", limit=100, cursor=page["next_cursor"])["result"]
            faces.extend(page["items"])
        if all(any(row["definition_id"] == part["definition_id"] and row["geometry"]["surface_kind"] == "plane" and row["geometry"]["unit_normal"][2] * (1 if part["name"]["text"] == "bottom" else -1) > 0.99 for row in faces) for part in occurrences):
            break
        assert time.monotonic() < deadline, faces
        threading.Event().wait(0.1)
    def target(name, sign):
        part = next(row for row in occurrences if row["name"]["text"] == name)
        face = next(row for row in faces if row["definition_id"] == part["definition_id"] and row["geometry"]["surface_kind"] == "plane" and row["geometry"]["unit_normal"][2] * sign > 0.99 and abs(row["geometry"]["centroid_mm"][2] - (18 if sign > 0 else 0)) < 1e-6)
        return {"entity_id": face["id"], "instance_path": {"root_occurrence_id": part["id"], "steps": []}}
    targets = [target("bottom", 1), target("shelf", -1)]
    for mode in ["minimum", "supporting_planes"]:
        kwargs = {"direction": [0, 0, 1]} if mode == "supporting_planes" else {}
        measured = mcp("inspect", action="measure", expected=stamp, faces=targets, mode=mode, **kwargs)
        assert measured["result"]["state"] == "verified"
        assert measured["result"]["distance_mm"] == pytest.approx(650)
        assert measured["result"]["unit"] == "mm"
        assert measured["result"]["canonical_mutation"] is False
        assert measured["stamp"] == stamp
    checked = mcp("program", action="validate", expected=stamp)
    assert checked["result"]["validation"]["state"] == "passed", checked
    assert checked["stamp"] == stamp
    invalid = [dict(targets[0], entity_id=9223372036854775807), targets[1]]
    mcp("inspect", error=True, action="measure", expected=stamp, faces=invalid, mode="minimum")
    after = mcp("inspect", action="status")
    assert after["stamp"] == stamp
    for field in ["undo_steps", "redo_steps", "selection"]:
        assert after["result"][field] == before["result"][field]
    assert mcp("program", action="read")["result"]["source"] == source
    mcp("program", action="patch", expected=stamp, edits=[{"old": "668", "new": "678"}])
    mcp("inspect", error=True, action="measure", expected=stamp, faces=targets, mode="minimum")
    mcp("edit", action="undo")
    restored = mcp("program", action="validate")
    assert restored["result"]["exact_collisions"]["measurements"][0]["distance_mm"] == pytest.approx(650)
    saved = tmp_path / "measured.ketchup"
    mcp("file", action="save_as", path=str(saved))
    assert saved.is_file()
    mcp("file", action="open", path=str(saved))
    assert mcp("program", action="read")["result"]["source"] == source


def test_modular_cabinet_local_edit_and_complete_reports(mcp, tmp_path):
    shelves = mcp("program", action="docs", name="shared-partition-shelves.star")
    checked_shelves = mcp("program", action="apply", source=shelves["text"])
    assert checked_shelves["result"]["validation"]["state"] == "passed", checked_shelves
    example = mcp("program", action="docs", name="modular-doweled-cabinet.star")
    source = example["text"]
    applied = mcp("program", action="apply", file_name="modular-doweled-cabinet.star", source=source)
    assert applied["result"]["validation"]["state"] == "passed", applied
    assert applied["result"]["report"]["bom"]["total_parts"] == 88
    page = mcp("inspect", action="query", kind="occurrences", limit=100)["result"]
    occurrences = list(page["items"])
    while page["next_cursor"] is not None:
        page = mcp("inspect", action="query", kind="occurrences", limit=100, cursor=page["next_cursor"])["result"]
        occurrences.extend(page["items"])
    caps = [row["id"] for row in occurrences if row["name"]["text"].startswith("upper ") and row["name"]["text"].endswith(" top")]
    assert len(caps) == 4
    mcp("view", action="selection", occurrence_ids=caps)
    selection = mcp("program", action="read", mode="selection")["result"]
    assert len(selection["parts"]) == 4, selection
    assert "source" not in selection
    assert any("top = board" in line["text"] for line in selection["lines"])
    for before_cap, after_cap in [("True", "False"), ("False", "True")]:
        context = mcp("program", action="read")
        cap_result = mcp("program", action="patch", expected=context["stamp"], edits=[{"old": "upper_cap = " + before_cap, "new": "upper_cap = " + after_cap}])
        assert cap_result["result"]["validation"]["state"] == "passed", cap_result
        assert cap_result["result"]["report"]["bom"]["total_parts"] == 88
    connector = "\nh1=hole('lower 0 right','x+',at=(260,300),diameter=8,depth=18,through=True,id='module-connector')\nh2=hole('lower 1 left','x-',at=(260,300),diameter=8,depth=18,through=True,id='module-connector')\njoint('lower 0 right','lower 1 left',kind='connector',fastener='M6 connector',fasteners=[(875,260,380)],links=[joint_link([h1,h2])])\n"
    linked = mcp("program", action="apply", source=source + connector)
    assert linked["result"]["validation"]["state"] == "passed", linked
    removed = mcp("program", action="patch", expected=linked["stamp"], edits=[{"old": connector, "new": ""}])
    assert removed["result"]["validation"]["state"] == "passed", removed
    assert mcp("program", action="read")["result"]["source"] == source
    old, new = "right_clearance = 650", "right_clearance = 660"
    # Compare the old full-read/full-write workflow with source-only/read + patch.
    # Both start at the same geometry and perform the same real edit with exact checks.
    measurements = {}
    for workflow in ["full", "patch"]:
        start = len(mcp.metrics)
        context = mcp("program", action="read", mode="full" if workflow == "full" else "source")
        assert context["result"]["source"] == source
        stamp = context["stamp"]
        if workflow == "full":
            result = mcp("program", action="apply", expected=stamp, source=source.replace(old, new))
        else:
            result = mcp("program", action="patch", expected=stamp, edits=[{"old": old, "new": new}])
        calls = mcp.metrics[start:]
        measurements[workflow] = {"calls": len(calls), "bytes": sum(c["request_bytes"] + c["response_bytes"] for c in calls), "seconds": sum(c["seconds"] for c in calls)}
        assert len(calls) == 2
        assert result["result"]["validation"]["state"] == "passed", result
        assert any(m["distance_mm"] == pytest.approx(660) for m in result["result"]["exact_collisions"]["measurements"])
        mcp("edit", action="undo")
    assert measurements["patch"]["bytes"] < measurements["full"]["bytes"], measurements
    docs = {}
    for detail in ["implementation", "concise"]:
        start = len(mcp.metrics)
        mcp("program", action="docs", name="joinery", detail=detail)
        docs[detail] = mcp.metrics[start]["response_bytes"]
    assert docs["concise"] < docs["implementation"]
    (tmp_path / "communication-comparison.json").write_text(json.dumps({"workflow": measurements, "docs_bytes": docs, "note": "Same fixture/edit/current binary; full API mode emulates previous payload, not a historical-binary timing comparison. One run each, not an SLA benchmark."}, indent=2), encoding="utf-8")
    read = mcp("program", action="read")
    before = mcp("inspect", action="status")
    mcp("program", error=True, action="patch", expected=read["stamp"], edits=[{"old": "board(", "new": "box("}])
    assert mcp("inspect", action="status")["stamp"] == before["stamp"]
    # A no-mutation read or failed patch must retain redo of the actual edit.
    mcp("edit", action="redo")
    changed = mcp("program", action="read")
    assert changed["result"]["source"] == source.replace(old, new)
    mcp("program", error=True, action="patch", expected=read["stamp"], edits=[{"old": old, "new": new}])
    mcp("edit", action="undo")
    checked = mcp("program", action="validate")
    assert checked["result"]["validation"]["state"] == "passed", checked
    def rows(section):
        offset = 0
        values = []
        while True:
            page = mcp("program", action="report", expected=checked["stamp"], section=section, offset=offset, limit=50)["result"]
            values.extend(page["rows"])
            if page["next_offset"] is None:
                assert len(values) == page["total"]
                return values
            assert page["next_offset"] > offset
            offset = page["next_offset"]
    reports = {section: rows(section) for section in ["cut_list", "hardware", "machining", "relations", "issues"]}
    assert len({row["part"] for row in reports["cut_list"]}) == 88
    assert reports["hardware"] and all("dowel" in row["item"] for row in reports["hardware"])
    assert all(row["operation"]["through"] is False for row in reports["machining"])
    assert len(reports["relations"]) > 40
    assert not reports["issues"], reports["issues"]
    (tmp_path / "cabinet-bom.json").write_text(json.dumps(reports, indent=2), encoding="utf-8")
    saved = tmp_path / "cabinet.ketchup"
    mcp("file", action="save_as", path=str(saved))
    mcp("file", action="open", path=str(saved))
    assert mcp("program", action="read")["result"]["source"] == source
    reopened = mcp("program", action="validate")
    assert reopened["result"]["validation"]["state"] == "passed", reopened
    assert any(m["distance_mm"] == pytest.approx(650) for m in reopened["result"]["exact_collisions"]["measurements"])
