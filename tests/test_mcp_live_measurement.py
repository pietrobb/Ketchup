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
            if error is not None:
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


def test_validator_catalog_discovery_and_execution(mcp):
    before = mcp("inspect", action="status")
    catalog = mcp("list_validators")["result"]
    assert catalog["catalog_only"] is True
    assert mcp("inspect", action="status") == before
    rows = {row["id"]: row for row in catalog["validators"]}
    assert len(rows) == len(catalog["validators"]) == 15
    assert {row["id"] for row in rows.values() if row["kind"] == "document"} == set(catalog["document_edit_route"]["supported_ids"])
    docs = catalog["documentation"]
    assert mcp(docs["tool"], **docs["arguments"])["text"]
    mcp("program", action="apply", file_name="catalog.star", source="a=box('base',(2,2,2),at=(-100,0,0))\nb=box('moving',(2,2,2))\njoint(a,b,kind='motion',name='travel',motion=slide((0,1,0),0,100))\nassembly_step('travel',start=100,end=0)\nt=box('holder',(4,4,8),at=(200,0,10),tool=True)\ntool_access('approach',envelope=t,motion=slide((0,0,1),0,20),start=20)")
    before = mcp("inspect", action="status")
    assert mcp("list_validators")["result"] == catalog
    for row in rows.values():
        run = row["run"]
        reply = mcp(run["tool"], **run["arguments"])
        detail = reply
        for key in row["result_path"].split("."):
            detail = detail[key]
        assert detail["state"] in {"passed", "failed", "not_evaluated", "incomplete"}, (row, detail)
        assert reply["result"]["canonical_mutation"] is False
        if row["id"] in {"program_geometry", "collision", "motion", "assembly_path", "tool_access"}:
            assert detail["state"] == "passed", (row, detail)
        assert mcp("inspect", action="status") == before
    # Use a discovered validator to distinguish missing evidence, success and a real failure.
    row = rows["beam_deflection"]
    assert row["required_roles"] == ["physics.beam.{xy|xz|yz}"]
    for material, state in [("unknown", "not_evaluated"), ("steel", "passed"), ("engineered_wood", "failed")]:
        mcp("program", action="apply", file_name="catalog.star", source=f"box('plate',(1000,300,20),material='{material}',attributes={{'classification:ketchup.validator-role.v1':'physics.beam.xy'}})")
        before = mcp("inspect", action="status")
        reply = mcp(row["run"]["tool"], **row["run"]["arguments"])
        detail = reply["result"]["validation"]["document_checks"]["beam_deflection"]
        assert detail["state"] == state, detail
        if state == "not_evaluated":
            assert detail["complete"] is False
        assert mcp("inspect", action="status") == before


def test_tool_access_through_public_mcp(mcp, tmp_path):
    geometry = "box('work',(20,20,2),at=(-10,-10,-4))\nt=box('holder',(4,4,8),at=(-2,-2,0),tool=True)\n"
    declaration = "tool_access('approach',envelope=t,motion=slide((0,0,1),0,20),start=20)\n"
    obstacle = "box('lip',(2,4,2),at=(1,-2,14))\n"
    for suffix, state in [("", "incomplete"), ("tool_access('missing')", "incomplete"), (declaration, "passed"), (declaration + obstacle, "failed")]:
        source = geometry + suffix
        mcp("program", action="apply", file_name="access.star", source=source)
        before = mcp("inspect", action="status")
        result = mcp("program", action="validate", expected=before["stamp"], validators=["tool_access"])["result"]
        check = result["validation"]["tool_access"]
        assert check["state"] == state, result
        assert result["validation"]["state"] == state, result
        assert result["canonical_mutation"] is False
        assert mcp("inspect", action="status") == before
        assert mcp("program", action="read")["result"]["source"] == source
        parts = mcp("inspect", action="query", kind="occurrences", limit=100)["result"]["items"]
        assert all(p["name"]["text"] != "holder" for p in parts)
        if state == "passed":
            mixed = mcp("program", action="validate", validators=["tool_access", "unknown_validator"])["result"]
            assert mixed["validation"]["tool_access"]["state"] == "passed", mixed
            assert mixed["validation"]["state"] == "incomplete", mixed
        if state == "failed":
            hit = check["checks"][0]["pairs"][0]
            assert hit["obstacle"]["name"] == "lip"
            assert hit["issues"][0]["common_volume_mm3"] > 0
    mcp("edit", action="undo")
    assert mcp("program", action="validate", validators=["tool_access"])["result"]["validation"]["tool_access"]["state"] == "passed"
    mcp("edit", action="redo")
    path = str(tmp_path / "access.ketchup")
    mcp("file", action="save_as", path=path)
    mcp("file", action="open", path=path)
    assert mcp("program", action="validate", validators=["tool_access"])["result"]["validation"]["tool_access"]["state"] == "failed"


@pytest.mark.parametrize("rotated", [False, True])
def test_tool_access_aperture_through_public_mcp(mcp, rotated):
    for width, state in [(2, "passed"), (6, "failed")]:
        source = f"a=box('left',(10,20,2),at=(-12,-10,10))\nb=box('right',(10,20,2),at=(2,-10,10))\nt=box('holder',({width},2,4),at=(-{width}/2,-1,0),tool=True)\n"
        if rotated:
            source += "rotate(t,axis=(0,1,0),angle=90,pivot=(0,0,0))\nrotate(a,axis=(0,1,0),angle=90,pivot=(0,0,0))\nrotate(b,axis=(0,1,0),angle=90,pivot=(0,0,0))\n"
        axis = "(1,0,0)" if rotated else "(0,0,1)"
        source += f"tool_access('aperture',envelope=t,motion=slide({axis},0,20),start=20)"
        mcp("program", action="apply", file_name="access.star", source=source)
        before = mcp("inspect", action="status")
        result = mcp("program", action="validate", validators=["tool_access"])["result"]
        assert result["validation"]["tool_access"]["state"] == state, result
        assert mcp("inspect", action="status") == before


def test_ordered_assembly_paths_through_public_mcp(mcp, tmp_path):
    geometry = "a=box('base',(2,2,2),at=(-100,0,0))\nb=box('insert',(2,2,2),at=(20,0,0))\nc=box('stop',(2,2,2),at=(10,0,0))\njoint(a,b,kind='motion',name='insert',motion=slide((1,0,0),-20,0))\njoint(a,c,kind='motion',name='stop',motion=slide((0,1,0),0,20))\n"
    insert = "assembly_step('insert',start=-20,end=0)\n"
    stop = "assembly_step('stop',start=20,end=0)\n"
    for suffix, state in [("", "incomplete"), (insert + stop, "passed"), (stop + insert, "failed")]:
        source = geometry + suffix
        mcp("program", action="apply", file_name="assembly.star", source=source)
        before = mcp("inspect", action="status")
        result = mcp("program", action="validate", expected=before["stamp"], validators=["assembly_path"])["result"]
        check = result["validation"]["assembly_path"]
        assert check["state"] == state, result
        assert result["validation"]["state"] == state, result
        assert result["canonical_mutation"] is False
        assert mcp("inspect", action="status") == before
        assert mcp("program", action="read")["result"]["source"] == source
        if state == "passed":
            mixed = mcp("program", action="validate", validators=["assembly_path", "unknown_validator"])["result"]
            assert mixed["validation"]["assembly_path"]["state"] == "passed", mixed
            assert mixed["validation"]["state"] == "incomplete", mixed
        if state == "failed":
            hit = check["steps"][1]["pairs"][0]
            assert hit["moving"]["name"] == "insert"
            assert hit["obstacle"]["name"] == "stop"
            assert hit["issues"][0]["common_volume_mm3"] > 7.99
    mcp("edit", action="undo")
    assert mcp("program", action="validate", validators=["assembly_path"])["result"]["validation"]["assembly_path"]["state"] == "passed"
    mcp("edit", action="redo")
    path = str(tmp_path / "assembly.ketchup")
    mcp("file", action="save_as", path=path)
    mcp("file", action="open", path=path)
    assert mcp("program", action="validate", validators=["assembly_path"])["result"]["validation"]["assembly_path"]["state"] == "failed"
    # Supported contact needs a swept-hull proof; unsupported contact remains incomplete.
    contact = "a=box('base',(2,2,2))\nb=box('insert',(2,2,2),at=(2,0,0))\njoint(a,b,kind='motion',name='insert',motion=slide((1,0,0),0,10))\nassembly_step('insert',start=10,end=0)"
    mcp("program", action="apply", file_name="assembly.star", source=contact)
    before = mcp("inspect", action="status")
    result = mcp("program", action="validate", validators=["assembly_path"])["result"]
    assert result["validation"]["assembly_path"]["state"] == "passed", result
    pair = result["validation"]["assembly_path"]["steps"][0]["pairs"][0]
    assert any(i.get("method") == "translation_swept_hull_with_native_contact_tolerance" for i in pair["verified_intervals"])
    assert mcp("inspect", action="status") == before
    rounded = contact.replace("box('insert',(2,2,2),at=(2,0,0))", "extrude('insert',distance=2,profile=round_corners([[2,0],[4,0],[4,2],[2,2]],0.25))")
    mcp("program", action="apply", file_name="assembly.star", source=rounded)
    before = mcp("inspect", action="status")
    result = mcp("program", action="validate", validators=["assembly_path"])["result"]
    assert result["validation"]["assembly_path"]["state"] == "incomplete", result
    assert mcp("inspect", action="status") == before


def test_continuous_motion_through_public_mcp(mcp):
    source = "a=box('a',(2,2,2),at=(-100,0,0))\nb=box('b',(2,2,2))\nbox('obstacle',(2,2,2),at=(10,0,0))\njoint(a,b,kind='motion',name='travel',motion=slide((1,0,0),0,20))\n"
    mcp("program", action="apply", file_name="motion.star", source=source)
    before = mcp("inspect", action="status")
    for end, state in [(3, "passed"), (20, "failed"), (21, "incomplete")]:
        result = mcp("program", action="validate", expected=before["stamp"], motion={"name":"travel", "from":0, "to":end})["result"]
        assert result["validation"]["motion"]["state"] == state, result
        assert result["validation"]["state"] == state, result
        assert result["canonical_mutation"] is False
        if state == "failed":
            hit = next(p for p in result["validation"]["motion"]["pairs"] if p["state"] == "failed")
            assert hit["obstacle"]["name"] == "obstacle"
            assert hit["issues"][0]["position"] == pytest.approx(10)
    assert mcp("inspect", action="status") == before
    assert mcp("program", action="read")["result"]["source"] == source


@pytest.mark.parametrize("case", ["rotation", "nested", "cavity", "cavity_with_retained_part"])
def test_exact_motion_geometry_through_public_mcp(mcp, case):
    if case == "rotation":
        source = "a=box('a',(2,2,2),at=(-100,0,0))\nb=box('b',(2,2,2),at=(10,0,0))\nbox('obstacle',(2,2,2),at=(-2,10,0))\njoint(a,b,kind='motion',name='travel',motion=rotate_motion((0,0,1),0,360))"
        end, state, position = 360, "failed", 90
    elif case == "nested":
        source = "a=box('a',(2,2,2),at=(-100,0,0))\nb=box('b',(2,2,2))\nc=component('base',[a,b])\ni=instance('copy',c,at=(100,0,0),x=(0,1,0))\njoint(a,i,kind='motion',name='travel',motion=slide((1,0,0),0,20),position=3)\nbox('obstacle',(2,2,2),at=(108,0,0))"
        end, state, position = 20, "failed", 10
    else:
        source = "a=box('obstacle',(20,20,10))\nc=box('cut',(16,16,12),at=(2,2,-1),tool=" + str(case == "cavity") + ")\nsubtract(a,c)\nb=box('b',(2,2,2),at=(4,4,4))\njoint(a,b,kind='motion',name='travel',motion=slide((1,0,0),0,8))"
        end, state, position = (8, "passed", None) if case == "cavity" else (8, "failed", 4)
    mcp("program", action="apply", file_name="motion.star", source=source)
    before = mcp("inspect", action="status")
    result = mcp("program", action="validate", expected=before["stamp"], motion={"name":"travel", "from":0, "to":end})["result"]
    check = result["validation"]["motion"]
    assert check["state"] == state, result
    if position is not None:
        hit = next(p for p in check["pairs"] if p["state"] == "failed")
        assert hit["issues"][0]["position"] == pytest.approx(position)
        if case == "nested":
            assert hit["moving"]["name"] == "copy/b"
            assert hit["moving"]["instance_path"]["steps"]
    else:
        assert check["complete"] is True
    assert mcp("inspect", action="status") == before
    assert mcp("program", action="read")["result"]["source"] == source


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


def test_program_scalar_loads_drive_read_only_validation_and_survive_reopen(mcp, tmp_path):
    source = "box('load',(10,10,10),attributes={'classification:ketchup.validator-role.v1':'physics.static.load:test','classification:ketchup.static-load-mode.v1':'compression','input:physics.mass_kg.occurrence.{occurrence}':'100','input:physics.applied_load_n.occurrence.{occurrence}':'200','input:physics.gravity_x_m_s2':'0','input:physics.gravity_y_m_s2':'0','input:physics.gravity_z_m_s2':'-9.81'})\nbox('support',(10,10,10),at=(20,0,0),attributes={'classification:ketchup.validator-role.v1':'physics.static.support:test','classification:ketchup.support-capacity.v1':'{\"source\":\"synthetic test capacity, not design data\",\"units\":\"N\",\"mode\":\"compression\",\"direction_world\":[0,0,-1],\"assumptions\":\"test fixture only\",\"additive\":false}','input:physics.support_capacity_n.occurrence.{occurrence}':'2000'})"
    for current, state, reason in [
        (source, "passed", None),
        (source.replace('synthetic test capacity, not design data', ''), "incomplete", "missing_capacity_source_or_assumptions"),
        (source.replace('[0,0,-1]', '[0,0,1]'), "incomplete", "uncovered_load_direction"),
        (source.replace('"mode":"compression"', '"mode":"pullout"'), "incomplete", "uncovered_load_mode"),
        (source.replace('"units":"N"', '"units":"kg"'), "incomplete", "unsupported_capacity_units"),
        (source.replace('classification:ketchup.support-capacity.v1', 'unqualified-capacity'), "incomplete", "missing_capacity_qualification"),
        (source.replace("'2000'", "'500'"), "failed", None),
        (source.replace("'input:physics.mass_kg.occurrence.{occurrence}':'100',", ""), "incomplete", "missing_or_ambiguous_mass"),
    ]:
        mcp("program", action="apply", file_name="loads.star", source=current)
        before = mcp("inspect", action="status")
        checked = mcp("program", action="validate", validators=["static_load"], expected=before["stamp"])
        assert checked["result"]["validation"]["state"] == state, checked
        detail = checked["result"]["validation"]["document_checks"]["static_load"]
        if state == "incomplete":
            assert any(row["reason"] == reason for row in detail["not_evaluated"]), detail
            assert not detail["evaluations"], detail
        else:
            measured = detail["evaluations"][0]
            assert measured["name"] == "load"
            assert measured["resultant_force_n"] == pytest.approx(1181)
            assert measured["supports"][0]["name"] == "support"
            evidence = measured["supports"][0]["qualification"]
            assert evidence["source"] == "synthetic test capacity, not design data"
            assert evidence["mode"] == "compression" and evidence["units"] == "N"
            assert evidence["direction_world"] == [0, 0, -1]
            assert evidence["basis"] == "user_declared_capacity_not_independently_verified"
            assert measured["capacity_margin_n"] == pytest.approx(819 if state == "passed" else -681)
        after = mcp("inspect", action="status")
        assert after["stamp"] == checked["stamp"] == before["stamp"]
        for field in ["undo_steps", "redo_steps", "selection"]:
            assert after["result"][field] == before["result"][field]
        assert mcp("program", action="read")["result"]["source"] == current
    mcp("edit", action="undo")
    assert mcp("program", action="validate", validators=["static_load"])["result"]["validation"]["state"] == "failed"
    mcp("edit", action="redo")
    saved = tmp_path / "loads.ketchup"
    mcp("file", action="save_as", path=str(saved))
    mcp("file", action="open", path=str(saved))
    assert mcp("program", action="read")["result"]["source"] == current
    assert mcp("program", action="validate", validators=["static_load"])["result"]["validation"]["state"] == "incomplete"
    repaired = mcp("program", action="apply", source=source)
    assert repaired["result"]["added_total"] == repaired["result"]["removed_total"] == 0
    before = mcp("inspect", action="status")
    checked = mcp("program", action="validate", validators=["static_load"])
    assert checked["result"]["validation"]["state"] == "passed", checked
    assert checked["stamp"] == before["stamp"]
    mcp("edit", action="undo")
    assert mcp("program", action="validate", validators=["static_load"])["result"]["validation"]["state"] == "incomplete"
    mcp("edit", action="redo")
    mcp("file", action="save")
    mcp("file", action="open", path=str(saved))
    checked = mcp("program", action="validate", validators=["static_load"])
    assert checked["result"]["validation"]["state"] == "passed", checked
    evidence = checked["result"]["validation"]["document_checks"]["static_load"]["evaluations"][0]["supports"][0]["qualification"]
    assert evidence["source"] == "synthetic test capacity, not design data"


def test_mixed_nested_validation_names_unchecked_parts_without_hiding_failures(mcp):
    source = "a=box('shelf',(1000,300,20),material='steel',attributes={'classification:ketchup.validator-role.v1':'physics.beam.xy'})\nc=component('shared',[a])\ninstance('copy',c,at=(2000,0,0))\nbox('root',(1000,300,20),at=(4000,0,0),material='steel',attributes={'classification:ketchup.validator-role.v1':'physics.beam.xy'})"
    for material, state in [("steel", "incomplete"), ("engineered_wood", "failed")]:
        mcp("program", action="apply", file_name="mixed.star", source=source.replace("steel", material))
        before = mcp("inspect", action="status")
        checked = mcp("program", action="validate", validators=["beam_deflection"], expected=before["stamp"])
        assert checked["result"]["validation"]["state"] == state, checked
        detail = checked["result"]["validation"]["document_checks"]["beam_deflection"]
        assert detail["complete"] is False
        assert [row["name"] for row in detail["evaluations"]] == ["root"]
        assert any("shelf" in row.get("name", "") and row["instance_path"]["steps"] for row in detail["not_evaluated"]), detail
        assert mcp("inspect", action="status")["stamp"] == before["stamp"]


def test_program_materials_drive_requested_read_only_validators(mcp, tmp_path):
    source = "box('shelf',(1000,300,20),material='steel',attributes={'classification:ketchup.validator-role.v1':'physics.beam.xy'})"
    for material, state in [("steel", "passed"), ("engineered_wood", "failed"), ("unknown", "incomplete")]:
        current = source.replace("'steel'", repr(material))
        mcp("program", action="apply", file_name="structural.star", source=current)
        before = mcp("inspect", action="status")
        checked = mcp("program", action="validate", validators=["beam_deflection"], expected=before["stamp"])
        assert checked["stamp"] == before["stamp"]
        assert checked["result"]["validation"]["state"] == state, checked
        detail = checked["result"]["validation"]["document_checks"]["beam_deflection"]
        assert detail["state"] == ("not_evaluated" if state == "incomplete" else state), detail
        if material != "unknown":
            assert detail["evaluations"][0]["material"] == material
            assert detail["evaluations"][0]["span_mm"] == pytest.approx(1000)
            assert detail["evaluations"][0]["predicted_deflection_mm"] > 0
        after = mcp("inspect", action="status")
        assert after["stamp"] == before["stamp"]
        for field in ["undo_steps", "redo_steps", "selection"]:
            assert after["result"][field] == before["result"][field]
        assert mcp("program", action="read")["result"]["source"] == current
    mcp("edit", action="undo")
    assert mcp("program", action="validate", validators=["beam_deflection"])["result"]["validation"]["state"] == "failed"
    mcp("edit", action="redo")
    saved = tmp_path / "structural.ketchup"
    mcp("file", action="save_as", path=str(saved))
    mcp("file", action="open", path=str(saved))
    assert mcp("program", action="read")["result"]["source"] == current
    assert mcp("program", action="validate", validators=["beam_deflection"])["result"]["validation"]["state"] == "incomplete"
    for names in [[], ["unknown_validator"]]:
        checked = mcp("program", action="validate", validators=names)
        assert checked["result"]["validation"]["state"] == "incomplete", checked


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


def test_house_layers_views_and_section_keep_the_program_through_public_mcp(mcp, tmp_path):
    house = Path(__file__).resolve().parents[1] / "examples" / "programs" / "tiny-house.star"
    mcp("program", action="apply", source_path=str(house), replace_document=True)
    def tags():
        return {tag["name"]["text"]: tag for tag in mcp("inspect", action="summary")["result"]["tags"]}
    ids = {name: tag["id"] for name, tag in tags().items()}
    assert {"koncept", "konštrukcia", "stĺpiky", "izolácia", "OSB", "krokvy"} <= set(ids)
    def show_only(shown, hidden):
        for name in (shown, hidden):
            result = mcp("view", action="tag_visibility", name=name, visible=name == shown)["result"]
            assert result["program_owned"] is True and result["visible"] is (name == shown), result
    def visible(name):
        return tags()[name]["visible"]

    show_only("koncept", "konštrukcia")
    mcp("view", action="save_view", name="Koncept")
    show_only("konštrukcia", "koncept")
    cut = mcp("view", action="section", normal=[0, 0, 1], offset_mm=1500)["result"]
    assert cut["canonical_mutation"] is False and cut["section"], cut
    listed = mcp("view", action="save_view", name="Konštrukcia v reze")["result"]
    assert [view["name"] for view in listed["saved_views"]] == ["Koncept", "Konštrukcia v reze"]
    assert listed["saved_views"][1]["hidden_tags"] == ["koncept"] and listed["saved_views"][1]["section"]
    mcp("view", action="close_section")

    # New dimensions recompute both representations; the views still switch between them.
    longer = mcp("program", action="apply", source_path=str(house), overrides={"length": 7400})
    # Only the rafter trimmed around the roof window moves to the rafter now under it.
    removed = longer["result"]["removed"]
    assert longer["result"]["removed_total"] == len(removed) <= 4, longer["result"]
    assert all("/krokva " in name for name in removed), removed
    framing, cursor = [], None
    while True:
        page = mcp("inspect", action="query", kind="occurrences", tag_id=ids["konštrukcia"], limit=100, **({"cursor": cursor} if cursor else {}))["result"]
        framing += page["items"]
        cursor = page["next_cursor"]
        if not cursor:
            break
    assert len(framing) > 100 and all(row["name"]["text"].startswith("konštrukcia/") for row in framing)
    for name, shown, hidden in [("Koncept", "koncept", "konštrukcia"), ("Konštrukcia v reze", "konštrukcia", "koncept")]:
        result = mcp("view", action="show_view", name=name)["result"]
        assert result["program_owned"] is True, result
        assert visible(shown) and not visible(hidden), name
    assert mcp("program", action="read")["result"]["overrides"] == {"length": 7400}

    saved = tmp_path / "house.ketchup"
    mcp("file", action="save_as", path=str(saved))
    mcp("file", action="open", path=str(saved))
    views = mcp("view", action="saved_views")["result"]
    assert [view["name"] for view in views["saved_views"]] == ["Koncept", "Konštrukcia v reze"]
    mcp("view", action="show_view", name="Koncept")
    assert visible("koncept") and not visible("konštrukcia")


def test_house_project_drawings_pdf_with_title_block_through_public_mcp(mcp, tmp_path):
    house = Path(__file__).resolve().parents[1] / "examples" / "programs" / "tiny-house.star"
    mcp("program", action="apply", source_path=str(house), replace_document=True)
    mcp("view", action="tag_visibility", name="koncept", visible=False)
    sheet = tmp_path / "domcek.pdf"
    title_block = {"client": "Ján Novák", "location": "Žilina, parc. č. 1234/5", "drawing_number": "D.1.01", "stage": "DSP"}
    def export(**arguments):
        # The sheet waits for the exact evaluation of every visible part.
        deadline = time.monotonic() + 300
        while True:
            result = mcp("file", action="export_drawings", path=str(sheet), error=None, **arguments)
            if result.get("error") != "drawings_unavailable" or time.monotonic() > deadline:
                return result["result"]
            time.sleep(2)

    result = export(format="auto", title_block=title_block)
    assert result["exported"] is True and result["scale"] == "1:50", result
    assert result["title_block"]["client"] == "Ján Novák" and result["dirty"] is True, result
    cuts = {view["view"]: view["cut_solids"] for view in result["views"]}
    assert cuts["plan"] > 50 and cuts["longitudinal_section"] > 0 and cuts["cross_section"] > 0, cuts
    import fitz
    document = fitz.open(sheet)
    text = document[0].get_text()
    for expected in ["Ján Novák", "Žilina, parc. č. 1234/5", "D.1.01", "1:50", result["format"], "Longitudinal section"]:
        assert expected in text, expected
    assert mcp("file", action="export_drawings", path=str(tmp_path / "sheet.svg"), error=True)["error"] == "invalid_path"
    assert mcp("file", action="export_drawings", path=str(sheet), format="B5", error=True)["error"] == "invalid_sheet_format"

    # The title block is kept in the document.
    saved = tmp_path / "house.ketchup"
    mcp("file", action="save_as", path=str(saved))
    mcp("file", action="open", path=str(saved))
    again = export()
    assert again["title_block"]["location"] == "Žilina, parc. č. 1234/5" and again["dirty"] is False, again
