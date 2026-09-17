"""Regression coverage for physical identity and complete machining transport."""
from copy import deepcopy

import pytest

from test_manufacturing_plugins import job
from ketchup.manufacturing import export_job, HomagWoodwopAdapter, ManufacturingError


@pytest.mark.parametrize("path", [
    {}, {"occurrences": [1]}, {"root_occurrence_id": 1},
    {"root_occurrence_id": True, "steps": []},
    {"root_occurrence_id": 0, "steps": []},
    {"root_occurrence_id": -1, "steps": []},
    {"root_occurrence_id": 1 << 64, "steps": []},
    {"root_occurrence_id": 1, "steps": None},
    {"root_occurrence_id": 1, "steps": [], "extra": 1},
    {"root_occurrence_id": 1, "steps": [{}]},
    {"root_occurrence_id": 1, "steps": [{"kind": "group", "id": 2}]},
    {"root_occurrence_id": 1, "steps": [{"kind": "unknown", "id": 2}]},
    {"root_occurrence_id": 1, "steps": [{"kind": "occurrence", "id": True}]},
    {"root_occurrence_id": 1, "steps": [{"kind": "occurrence", "id": 0}]},
    {"root_occurrence_id": 1, "steps": [{"kind": "occurrence", "id": 1 << 64}]},
    {"root_occurrence_id": 1, "steps": [{"kind": "occurrence", "id": 2, "extra": 1}]},
    {"root_occurrence_id": 1, "steps": [{"kind": "occurrence", "id": 2}] * 257},
])
def test_invalid_physical_identity_never_publishes(tmp_path, path):
    source = job()
    source["parts"][0]["instance_path"] = path
    with pytest.raises(ManufacturingError, match="instance_path"):
        export_job(source, tmp_path / "order", [], confirmed=True)
    assert not list(tmp_path.iterdir())


def test_nested_paths_remain_distinct_and_preserved(tmp_path):
    source = job()
    for i, part in enumerate(source["parts"]):
        part["instance_path"] = {"root_occurrence_id": 1, "steps": [
            {"kind": "group", "id": 2}, {"kind": "occurrence", "id": i + 3}]}
    result = export_job(source, tmp_path / "order", [], confirmed=True)
    assert result["job"] == source
    source["parts"][1]["instance_path"] = deepcopy(source["parts"][0]["instance_path"])
    with pytest.raises(ManufacturingError, match="duplicate physical"):
        export_job(source, tmp_path / "duplicate", [], confirmed=True)
    assert not (tmp_path / "duplicate").exists()


@pytest.mark.parametrize("dowel_only", [False, True])
def test_missing_machining_program_never_publishes(tmp_path, dowel_only):
    source = job()
    if dowel_only:
        source["parts"][0]["operations"] = [{"kind": "stock"}]
        source["parts"][0]["dowel_holes"] = [{"kind": "dowel_drill"}]
    source["outputs"]["homag-woodwop4"] = []
    with pytest.raises(ManufacturingError, match="missing HOMAG programs"):
        export_job(source, tmp_path / "order", [HomagWoodwopAdapter()], confirmed=True)
    assert not list(tmp_path.iterdir())


def test_extra_program_for_unmachined_piece_is_rejected():
    source = job()
    code = source["parts"][1]["code"]
    source["outputs"]["homag-woodwop4"].append({
        "code": code, "filename": code + ".mpr", "content": "obsolete program"})
    with pytest.raises(ManufacturingError, match="unexpected"):
        HomagWoodwopAdapter().render(source)


def test_short_cut_only_code_does_not_inherit_machine_restrictions():
    source = job()
    source["parts"][1]["code"] = "CUT-ONLY"
    assert list(HomagWoodwopAdapter().render(source)) == ["000000000000.mpr"]
    source["parts"][0]["operations"] = [{"kind": "stock"}]
    source["outputs"]["homag-woodwop4"] = []
    assert HomagWoodwopAdapter().render(source) == {}


@pytest.mark.parametrize("field,value", [
    ("operations", None), ("operations", []), ("operations", [{}]),
    ("operations", [{"kind": "circular-drill"}]),
    ("operations", [{"kind": "stock"}, {"kind": "stock"}]),
    ("operations", [{"kind": "stock"}, {"kind": ""}]),
    ("dowel_holes", None), ("dowel_holes", ["not a hole"]),
])
def test_missing_or_malformed_machining_metadata_fails_closed(field, value):
    source = job()
    source["parts"][0][field] = value
    with pytest.raises(ManufacturingError, match="explicit stock"):
        HomagWoodwopAdapter().render(source)
