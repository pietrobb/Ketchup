"""Offline manufacturing contract tests; no CNC/JAF service acceptance claimed."""
import builtins
from copy import deepcopy
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
from unittest.mock import patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "sdk" / "python"))
from ketchup.manufacturing import (
    export_job, HomagWoodwopAdapter, JafWebCutAdapter, ManufacturingError,
)


def job(count=2):
    parts = [{"instance_path": {"root_occurrence_id": i + 1, "steps": []}, "code": f"{i:012d}",
              "name": "=not_a_formula", "material_key": "oak", "dowel_holes": [], "stock_shape": "rectangular_prism",
              "dimensions_mm": [600, 400, 18], "operations": [{"kind": "stock"}] + ([{"kind": "circular-drill", "operation_id": "drill-A"}] if i == 0 else []),
              "machining_setups": [{"id": "A", "code": f"{i:012d}", "operation_ids": ["drill-A"], "dowel_hole_ids": []}] if i == 0 else []} for i in range(count)]
    return {"schema": "ketchup.production-job.v2", "document_id": 1,
            "source_revision": 2, "source_digest": "source-hash", "parts": parts,
            "outputs": {"homag-woodwop4": [
                {"part_code": parts[0]["code"], "setup_id": "A", "code": parts[0]["code"], "filename": parts[0]["code"] + ".mpr",
                 "content": '[H\r\nVERSION="4.0 Alpha"\r\n]\r\n'}] if parts else []}}


class MockAdapter:
    id = "other-machine"

    def __init__(self, files=None):
        self.files = files if files is not None else {"panel-program-v2.nc": b"G0 X1\n"}

    def render(self, source):
        return self.files


def test_neutral_disabled_plugins_and_audit(tmp_path):
    source = job()
    source["parts"][0]["code"] = "SHORT"
    source["parts"][0]["dimensions_mm"][0] = 1.25
    original = deepcopy(source)
    result = export_job(source, tmp_path / "new", [], confirmed=True)
    assert source == original
    assert result["job"] == source
    assert result["files"] == []
    assert list((tmp_path / "new").iterdir()) == [tmp_path / "new" / "manifest.json"]
    assert json.loads((tmp_path / "new" / "manifest.json").read_bytes()) == result


def test_mpr_exact_bytes_subset_and_second_adapter(tmp_path):
    source = job()
    result = export_job(source, tmp_path / "new", [HomagWoodwopAdapter(), MockAdapter()],
                        confirmed=True)
    expected = source["outputs"]["homag-woodwop4"][0]["content"].encode("ascii")
    assert (tmp_path / "new" / "000000000000.mpr").read_bytes() == expected
    assert not (tmp_path / "new" / "000000000001.mpr").exists()
    assert (tmp_path / "new" / "panel-program-v2.nc").read_bytes() == b"G0 X1\n"
    for record in result["files"]:
        content = (tmp_path / "new" / record["filename"]).read_bytes()
        assert record["sha256"] == hashlib.sha256(content).hexdigest()
        assert record["size_bytes"] == len(content)


@pytest.mark.parametrize("confirmed", [False, None, 1, "yes"])
def test_confirmation(tmp_path, confirmed):
    with pytest.raises(ManufacturingError, match="confirmed=True"):
        export_job(job(), tmp_path / "new", [], confirmed=confirmed)
    assert not list(tmp_path.iterdir())


@pytest.mark.parametrize("name", ["../escape", "a/b", "a\\b", "C:bad", "/tmp/file",
                                  "NUL.mpr", "CON", "foo.", "x\x00", "", "a b",
                                  "manifest.json", "MANIFEST.JSON"])
def test_invalid_filenames(tmp_path, name):
    with pytest.raises(ManufacturingError):
        export_job(job(), tmp_path / "new", [MockAdapter({name: b"x"})], confirmed=True)
    assert not list(tmp_path.iterdir())


def test_duplicates_and_invalid_adapter(tmp_path):
    for adapters in ([MockAdapter(), MockAdapter()], ["some.module"],
                     [MockAdapter({"a.NC": b"a", "A.nc": b"b"})],
                     [MockAdapter({"safe.nc": "not bytes"})]):
        with pytest.raises(ManufacturingError):
            export_job(job(), tmp_path / "new", adapters, confirmed=True)
    assert not list(tmp_path.iterdir())


@pytest.mark.parametrize("change", [
    lambda j: j.update(schema="wrong"), lambda j: j.update(parts=[]),
    lambda j: j.update(outputs=[]), lambda j: j.update(document_id=True),
    lambda j: j.update(source_digest=""), lambda j: j.update(source_revision=-1),
    lambda j: j["parts"][0].update(code="lower"),
    lambda j: j["parts"][0].update(code="A" * 65),
    lambda j: j["parts"][0].update(name=""),
    lambda j: j["parts"][0].update(instance_path=[]),
    lambda j: j["parts"][0].update(dimensions_mm=[1, 0, 3]),
    lambda j: j["parts"][0].update(dimensions_mm=[float("nan"), 2, 3]),
    lambda j: j["parts"][1].update(code=j["parts"][0]["code"]),
    lambda j: j["parts"][1].update(instance_path=j["parts"][0]["instance_path"]),
])
def test_invalid_job(tmp_path, change):
    source = job()
    change(source)
    with pytest.raises(ManufacturingError):
        export_job(source, tmp_path / "new", [], confirmed=True)
    assert not list(tmp_path.iterdir())


@pytest.mark.parametrize("change", [
    lambda j: j["parts"][0].update(code="SHORT"),
    lambda j: j["outputs"].clear(),
    lambda j: j["outputs"]["homag-woodwop4"].append(j["outputs"]["homag-woodwop4"][0]),
    lambda j: j["outputs"]["homag-woodwop4"][0].update(code="UNKNOWN00000"),
    lambda j: j["outputs"]["homag-woodwop4"][0].update(filename="different.mpr"),
    lambda j: j["outputs"]["homag-woodwop4"][0].update(content=""),
    lambda j: j["outputs"]["homag-woodwop4"][0].update(content="č"),
    lambda j: j["outputs"]["homag-woodwop4"][0].update(content="bad\x00"),
])
def test_invalid_homag(tmp_path, change):
    source = job()
    change(source)
    with pytest.raises(ManufacturingError):
        export_job(source, tmp_path / "new", [HomagWoodwopAdapter()], confirmed=True)
    assert not list(tmp_path.iterdir())


def test_overwrite_refused_including_empty_directory(tmp_path):
    target = tmp_path / "new"
    target.mkdir()
    with pytest.raises(FileExistsError):
        export_job(job(), target, [], confirmed=True)
    assert not list(target.iterdir())
    (target / "keep").write_bytes(b"original")
    with pytest.raises(FileExistsError):
        export_job(job(), target, [], confirmed=True)
    assert (target / "keep").read_bytes() == b"original"


def test_symlink_refusal(tmp_path):
    real = tmp_path / "real"
    real.mkdir()
    link = tmp_path / "link"
    try:
        link.symlink_to(real, target_is_directory=True)
    except OSError:
        pytest.skip("symlink creation not permitted on this host")
    with pytest.raises((ManufacturingError, FileExistsError)):
        export_job(job(), link, [], confirmed=True)
    with pytest.raises(ManufacturingError):
        export_job(job(), link / "new", [], confirmed=True)


def test_render_and_write_failure_leave_no_partial_directory(tmp_path):
    class Broken:
        id = "broken"
        def render(self, source):
            raise RuntimeError("render failure")
    with pytest.raises(RuntimeError):
        export_job(job(), tmp_path / "new", [MockAdapter(), Broken()], confirmed=True)
    with patch("ketchup.manufacturing.os.fsync", side_effect=OSError("disk failure")):
        with pytest.raises(OSError):
            export_job(job(), tmp_path / "new", [MockAdapter()], confirmed=True)
    assert not list(tmp_path.iterdir())


def test_publication_race_never_replaces_directory(tmp_path):
    from ketchup import manufacturing
    original = manufacturing._publish
    def raced(stage, destination):
        destination.mkdir()
        original(stage, destination)
    with patch.object(manufacturing, "_publish", side_effect=raced):
        with pytest.raises(OSError):
            export_job(job(), tmp_path / "new", [], confirmed=True)
    assert list(tmp_path.iterdir()) == [tmp_path / "new"]
    assert not list((tmp_path / "new").iterdir())


def test_lazy_dependency_in_fresh_interpreter(tmp_path):
    sdk = str(Path(__file__).resolve().parents[1] / "sdk" / "python")
    script = f'''import sys, builtins
sys.path.insert(0, {sdk!r})
original = builtins.__import__
def guarded(name, *a, **kw):
    if name == "openpyxl" or name.startswith("openpyxl."):
        raise ImportError("blocked optional dependency")
    return original(name, *a, **kw)
builtins.__import__ = guarded
from ketchup.manufacturing import export_job, HomagWoodwopAdapter, JafWebCutAdapter
source = {job()!r}
export_job(source, {str(tmp_path / 'neutral')!r}, [], confirmed=True)
export_job(source, {str(tmp_path / 'homag')!r}, [HomagWoodwopAdapter()], confirmed=True)
try:
    JafWebCutAdapter({{"oak": "DTDL"}}, allow_rotation=False).render(source)
except ImportError as exc:
    assert "optional openpyxl" in str(exc)
else:
    raise AssertionError("JAF did not require optional dependency")
assert "openpyxl" not in sys.modules
'''
    result = subprocess.run([sys.executable, "-c", script], capture_output=True, text=True)
    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize("rotation", [False, True])
def test_excel_physical_rows_text_codes_formulas_and_fractional_mm(tmp_path, rotation):
    openpyxl = pytest.importorskip("openpyxl")
    source = job()
    source["parts"][0]["dimensions_mm"] = [600.125, 400.25, 18.5]
    adapter = JafWebCutAdapter({"oak": "=MATERIAL"}, allow_rotation=rotation)
    export_job(source, tmp_path / "new", [adapter, HomagWoodwopAdapter()], confirmed=True)
    for data_only in (True, False):
        wb = openpyxl.load_workbook(tmp_path / "new" / adapter.filename, data_only=data_only)
        assert wb.sheetnames == ["všeobecný", "Export"]
        ws, ex = wb["všeobecný"], wb["Export"]
        for i in range(2):
            code = source["parts"][i]["code"]
            for cell in (ws.cell(18+i, 19), ex.cell(2+i, 14)):
                assert cell.value == code and cell.data_type == "s"
            for cell in (ws.cell(18+i, 20), ex.cell(2+i, 15)):
                assert cell.value == (code if i == 0 else None)
            assert ws.cell(18+i, 9).value == ex.cell(2+i, 5).value == 1
            assert ex.cell(2+i, 8).value == int(rotation)
            assert ws.cell(18+i, 8).data_type == "s"
            assert ex.cell(2+i, 4).data_type == "s"
        assert ws["F18"].data_type == ex["C2"].data_type == "s"
        assert ws["J18"].value == ex["F2"].value == 600.125
        assert ws["G18"].value == 18.5
        assert ws["D20"].value is None and ex["A4"].value is None
        if not data_only:
            assert ws["C18"].data_type == "f"
        wb.close()


def test_jaf_configuration_limits_and_capacity():
    openpyxl = pytest.importorskip("openpyxl")
    with pytest.raises(TypeError):
        JafWebCutAdapter({"oak": "DTDL"})
    adapter = JafWebCutAdapter({"oak": "DTDL"}, allow_rotation=False)
    with pytest.raises(ManufacturingError, match="mapping"):
        JafWebCutAdapter({}, allow_rotation=False).render(job())
    too_long = job()
    too_long["parts"][0]["code"] = "A" * 41
    with pytest.raises(ManufacturingError, match="40"):
        adapter.render(too_long)
    data = adapter.render(job(220))[adapter.filename]
    wb = openpyxl.load_workbook(io.BytesIO(data), data_only=True)
    assert wb["Export"]["N221"].value == "000000000219"
    wb.close()
    with pytest.raises(ManufacturingError, match="capacity"):
        adapter.render(job(221))
