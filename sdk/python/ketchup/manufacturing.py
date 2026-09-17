"""Opt-in manufacturing exports; adapters are explicitly supplied TRUSTED Python objects.

No plugin discovery/import by name occurs. The neutral manifest requires only the
standard library. JafWebCutAdapter alone lazily requires openpyxl>=3.1,<4.
Adapters must render without external side effects; this is not a Python sandbox.
"""
from __future__ import annotations

from collections.abc import Iterable, Mapping
from copy import deepcopy
import ctypes
import hashlib
import io
import json
import math
import os
from pathlib import Path
import re
import shutil
import stat
import sys
import tempfile
from typing import Protocol


class ManufacturingError(ValueError):
    """Invalid production job, adapter configuration or unsafe output."""


class ManufacturingAdapter(Protocol):
    id: str

    def render(self, job: dict) -> Mapping[str, bytes]: ...


def _text(value, label):
    if not isinstance(value, str) or not value.strip():
        raise ManufacturingError(f"{label} must be a nonempty string")
    return value


def _json(value):
    try:
        return json.dumps(value, ensure_ascii=True, allow_nan=False, sort_keys=True,
                          indent=2).encode("utf-8") + b"\n"
    except (TypeError, ValueError, RecursionError) as exc:
        raise ManufacturingError("job must contain finite JSON data") from exc


def _code(value, label):
    if not isinstance(value, str) or not re.fullmatch(r"[A-Z0-9_-]{1,64}", value):
        raise ManufacturingError(f"{label} must be 1..64 uppercase A-Z0-9_- characters")
    return value


def _ids(values, label):
    if not isinstance(values, list):
        raise ManufacturingError(f"{label} must be a list")
    result = set()
    for value in values:
        _text(value, label)
        if value in result:
            raise ManufacturingError(f"duplicate {label}: {value}")
        result.add(value)
    return result


def _validate_setups(part, program_codes):
    operations, holes = part.get("operations"), part.get("dowel_holes")
    if (not isinstance(operations, list) or not operations
            or any(not isinstance(op, dict) or not isinstance(op.get("kind"), str)
                   or not op["kind"].strip() for op in operations)
            or operations[0]["kind"] != "stock"
            or any(op["kind"] == "stock" for op in operations[1:])
            or not isinstance(holes, list) or any(not isinstance(h, dict) for h in holes)):
        raise ManufacturingError("production job requires explicit stock, operations and dowel_holes")
    operation_ids = _ids([op.get("operation_id") for op in operations[1:]], "operation IDs")
    hole_ids = _ids([hole.get("hole_id") for hole in holes], "dowel hole IDs")
    setups = part.get("machining_setups")
    if not isinstance(setups, list):
        raise ManufacturingError("machining_setups must be an explicit list")
    setup_ids, assigned_ops, assigned_holes = set(), set(), set()
    for setup in setups:
        if not isinstance(setup, dict):
            raise ManufacturingError("machining setup must be an object")
        ident = setup.get("id")
        if not isinstance(ident, str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", ident):
            raise ManufacturingError("invalid machining setup id")
        if ident in setup_ids:
            raise ManufacturingError("duplicate machining setup id")
        setup_ids.add(ident)
        code = _code(setup.get("code"), "setup program code")
        if code in program_codes:
            raise ManufacturingError(f"duplicate setup program code: {code}")
        program_codes.add(code)
        ops = _ids(setup.get("operation_ids"), "setup operation IDs")
        dowels = _ids(setup.get("dowel_hole_ids"), "setup dowel hole IDs")
        if (not (ops or dowels) or not ops <= operation_ids or not dowels <= hole_ids
                or ops & assigned_ops or dowels & assigned_holes):
            raise ManufacturingError("setup partition has empty, unknown or repeated machining")
        assigned_ops.update(ops)
        assigned_holes.update(dowels)
    if assigned_ops != operation_ids or assigned_holes != hole_ids:
        raise ManufacturingError("machining setups must cover every operation and dowel hole exactly once")


def _validate_job(job):
    if not isinstance(job, dict) or job.get("schema") != "ketchup.production-job.v2":
        raise ManufacturingError("expected ketchup.production-job.v2 job")
    for key in ("document_id", "source_revision"):
        if type(job.get(key)) is not int or job[key] < 0:
            raise ManufacturingError(f"{key} must be a nonnegative integer")
    _text(job.get("source_digest"), "source_digest")
    parts = job.get("parts")
    if not isinstance(parts, list) or not parts:
        raise ManufacturingError("parts must be a nonempty list")
    codes, paths, program_codes = set(), set(), set()
    for part in parts:
        if not isinstance(part, dict):
            raise ManufacturingError("part must be an object")
        code = _code(part.get("code"), "part code")
        if code in codes:
            raise ManufacturingError(f"duplicate part code: {code}")
        codes.add(code)
        identity = part.get("instance_path")
        if not isinstance(identity, dict) or set(identity) != {"root_occurrence_id", "steps"}:
            raise ManufacturingError("instance_path requires root_occurrence_id and steps")
        root = identity["root_occurrence_id"]
        if type(root) is not int or not 1 <= root <= (1 << 64) - 1:
            raise ManufacturingError("instance_path root must be a positive u64")
        steps = identity["steps"]
        if not isinstance(steps, list) or len(steps) > 256:
            raise ManufacturingError("instance_path steps must be a list of at most 256 steps")
        for step in steps:
            if (not isinstance(step, dict) or set(step) != {"kind", "id"}
                    or step["kind"] not in ("group", "occurrence")
                    or type(step["id"]) is not int or not 1 <= step["id"] <= (1 << 64) - 1):
                raise ManufacturingError("invalid instance_path step")
        if steps and steps[-1]["kind"] != "occurrence":
            raise ManufacturingError("instance_path must end at a physical occurrence, not a group")
        path = _json(identity)
        if path in paths:
            raise ManufacturingError("duplicate physical instance_path")
        paths.add(path)
        _text(part.get("name"), "part name")
        _text(part.get("material_key"), "material_key")
        dims = part.get("dimensions_mm")
        if (not isinstance(dims, list) or len(dims) != 3
                or any(type(d) not in (int, float) or not math.isfinite(d) or d <= 0
                       for d in dims)):
            raise ManufacturingError("dimensions_mm must contain positive finite length, width, thickness")
        _validate_setups(part, program_codes)
    if not isinstance(job.get("outputs"), dict):
        raise ManufacturingError("outputs must be an object keyed by adapter id")
    for adapter_id in job["outputs"]:
        _text(adapter_id, "output adapter id")
    _json(job)


def assign_setup_codes(job: dict, assignments: Mapping[str, Mapping[str, str]]) -> dict:
    """Rename existing setup codes, never split machining or infer a flip transform.

    assignments maps persistent part code -> setup id -> program code. Codes and
    bindings are saved in the immutable export manifest, NOT back into the CAD
    document. Built-in HOMAG outputs may be renamed (MPR bytes stay unchanged);
    other machine outputs must be regenerated by their trusted adapter.
    """
    _validate_job(job)
    if not isinstance(assignments, Mapping):
        raise ManufacturingError("setup code assignments must be a mapping")
    result = deepcopy(job)
    parts = {part["code"]: part for part in result["parts"]}
    for part_code, changes in assignments.items():
        if part_code not in parts or not isinstance(changes, Mapping):
            raise ManufacturingError("unknown part or invalid setup code mapping")
        setups = {setup["id"]: setup for setup in parts[part_code]["machining_setups"]}
        for setup_id, code in changes.items():
            if setup_id not in setups:
                raise ManufacturingError("unknown setup; assigning a code cannot create an orientation/program")
            setups[setup_id]["code"] = _code(code, "setup program code")
    _validate_job(result)
    if any(key != HomagWoodwopAdapter.id for key in result["outputs"]):
        raise ManufacturingError("assign codes before rendering other machine outputs")
    if HomagWoodwopAdapter.id in result["outputs"]:
        _homag_outputs(job)
        for output in result["outputs"][HomagWoodwopAdapter.id]:
            setup = next(s for s in parts[output["part_code"]]["machining_setups"]
                         if s["id"] == output["setup_id"])
            output["code"] = setup["code"]
            output["filename"] = setup["code"] + ".mpr"
        _homag_outputs(result)
    return result


def _filename(name):
    # Flat portable namespace: no directories, ADS, device names or normalization aliases.
    if (not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9_-][A-Za-z0-9_.-]{0,199}", name)
            or name.endswith(".") or name.split(".")[0].upper() in
            {"CON", "PRN", "AUX", "NUL", *(f"COM{i}" for i in range(10)),
             *(f"LPT{i}" for i in range(10))}):
        raise ManufacturingError(f"unsafe filename: {name!r}")
    return name


def _check_destination(destination):
    path = Path(destination).absolute()
    _filename(path.name)
    for parent in (path.parent, *path.parent.parents):
        info = parent.lstat()  # Missing parents are deliberately not created.
        if (stat.S_ISLNK(info.st_mode)
                or getattr(info, "st_file_attributes", 0) & 0x400):
            raise ManufacturingError("symlink/junction destination parents are forbidden")
        if not stat.S_ISDIR(info.st_mode):
            raise ManufacturingError("destination parent must be a directory")
    if os.path.lexists(path):
        raise FileExistsError(f"destination already exists: {path}")
    return path


def _publish(stage, destination):
    """Atomic no-replace directory rename (fail closed on unsupported platforms)."""
    if os.name == "nt":
        os.rename(stage, destination)  # Windows rename never replaces an existing target.
    elif sys.platform.startswith("linux"):
        libc = ctypes.CDLL(None, use_errno=True)
        rename = getattr(libc, "renameat2", None)
        if rename is None:
            raise ManufacturingError("atomic no-replace publication unavailable")
        rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p,
                           ctypes.c_uint]
        rename.restype = ctypes.c_int
        if rename(-100, os.fsencode(stage), -100, os.fsencode(destination), 1):
            error = ctypes.get_errno()
            raise OSError(error, os.strerror(error), str(destination))
    else:
        raise ManufacturingError("atomic no-replace publication unsupported on this platform")


def export_job(job: dict, destination, adapters: Iterable[ManufacturingAdapter], *,
               confirmed: bool = False) -> dict:
    """Publish a NEW directory and return its audit manifest (also manifest.json).

    confirmed must be literally True. Existing parents must be real directories,
    owned/trusted by the caller (not concurrently replaced by another process).
    All renders, validations and file writes precede an atomic no-replace rename.
    An empty adapters list exports only the neutral manifest. Unselected outputs
    are recorded in the source job but never written as machine files.
    """
    if confirmed is not True:
        raise ManufacturingError("manufacturing export requires explicit confirmed=True")
    _validate_job(job)
    snapshot = deepcopy(job)
    target = _check_destination(destination)
    selected = list(adapters)
    ids = set()
    for adapter in selected:
        ident = _text(getattr(adapter, "id", None), "adapter id")
        if ident in ids or not callable(getattr(adapter, "render", None)):
            raise ManufacturingError("duplicate adapter id or missing render(job)")
        ids.add(ident)
    files, names, records = {}, {"manifest.json"}, []
    for adapter in selected:
        rendered = adapter.render(deepcopy(snapshot))
        if not isinstance(rendered, Mapping):
            raise ManufacturingError("adapter render must return a filename-to-bytes mapping")
        for name, content in rendered.items():
            _filename(name)
            if name.casefold() in names:
                raise ManufacturingError(f"filename collision: {name}")
            if not isinstance(content, bytes):
                raise ManufacturingError(f"adapter output must be bytes: {name}")
            names.add(name.casefold())
            files[name] = content
            records.append({"adapter_id": adapter.id, "filename": name,
                            "size_bytes": len(content),
                            "sha256": hashlib.sha256(content).hexdigest()})
    manifest = {"schema": "ketchup.manufacturing-export.v1", "status": "completed",
                "job": snapshot, "job_sha256": hashlib.sha256(_json(snapshot)).hexdigest(),
                "adapters": [a.id for a in selected], "files": records}
    files["manifest.json"] = _json(manifest)
    stage = Path(tempfile.mkdtemp(prefix=".ketchup-export-", dir=target.parent))
    try:
        for name, content in files.items():
            with (stage / name).open("xb") as stream:
                stream.write(content)
                stream.flush()
                os.fsync(stream.fileno())
        _check_destination(target)
        _publish(stage, target)
    finally:
        if stage.exists():
            shutil.rmtree(stage)
    return manifest


def _homag_outputs(job, program_code_length=None):
    required = {(part["code"], setup["id"]): setup["code"]
                for part in job["parts"] for setup in part["machining_setups"]}
    if program_code_length is not None and any(len(code) != program_code_length for code in required.values()):
        raise ManufacturingError(f"HOMAG requires exactly {program_code_length} character setup program codes")
    outputs = job["outputs"].get(HomagWoodwopAdapter.id)
    if not isinstance(outputs, list):
        raise ManufacturingError("explicit homag-woodwop4 outputs list is required")
    result, seen = {}, set()
    for output in outputs:
        if not isinstance(output, dict):
            raise ManufacturingError("HOMAG output must be an object")
        part_code, setup_id = output.get("part_code"), output.get("setup_id")
        if not isinstance(part_code, str) or not isinstance(setup_id, str):
            raise ManufacturingError("unexpected HOMAG output without part_code and setup_id")
        key = part_code, setup_id
        if key not in required or key in seen or output.get("code") != required[key]:
            raise ManufacturingError("unexpected or duplicate HOMAG output setup/code binding")
        seen.add(key)
        name = output.get("filename")
        if name != required[key] + ".mpr":
            raise ManufacturingError("HOMAG filename must equal setup program code + .mpr")
        _filename(name)
        content = _text(output.get("content"), "MPR content")
        if any(ord(ch) > 127 or (ord(ch) < 32 and ch not in "\r\n\t")
               or ord(ch) == 127 for ch in content):
            raise ManufacturingError("MPR content must be ASCII text without control characters")
        result[name] = content.encode("ascii")
    if seen != required.keys():
        raise ManufacturingError(f"missing HOMAG programs for setups: {sorted(required.keys() - seen)}")
    return result


class HomagWoodwopAdapter:
    """Transport validated per-setup ASCII MPRs without rewriting their contents.

    Length is a destination rule, not a physical identity rule. Default remains
    12 until the operator explicitly selects another length confirmed by JAF.
    Coverage checks do not validate flip orientation, tools or machine safety.
    """
    id = "homag-woodwop4"

    def __init__(self, *, program_code_length: int = 12):
        if type(program_code_length) is not int or not 1 <= program_code_length <= 64:
            raise ManufacturingError("program_code_length must be 1..64")
        self.program_code_length = program_code_length

    def render(self, job: dict) -> Mapping[str, bytes]:
        _validate_job(job)
        return _homag_outputs(job, self.program_code_length)


class JafWebCutAdapter:
    """FurniGen template: S/N = physical ID, T/O = setup A, U/P = setup B.

    One physical part per row, including two-sided pieces. The second barcode
    field is opt-in pending confirmation of JAF's interpretation of U/P. Setup
    names other than A/B are not silently flattened. Cut-only rows have no
    machining barcode. This adapter neither plans nor transforms toolpaths.
    Export formulas are materialized as values, preserving leading zeroes.
    """
    id = "jaf-webcut"
    filename = "JAF_WebCut_v50.xlsx"

    def __init__(self, material_mapping: Mapping[str, str], *, allow_rotation: bool,
                 second_barcode_confirmed: bool = False):
        if not isinstance(material_mapping, Mapping) or type(allow_rotation) is not bool:
            raise ManufacturingError("explicit material mapping and boolean allow_rotation required")
        if type(second_barcode_confirmed) is not bool:
            raise ManufacturingError("second_barcode_confirmed must be a boolean")
        self.material_mapping = dict(material_mapping)
        for key, value in self.material_mapping.items():
            _text(key, "material key")
            _text(value, "JAF material")
        self.allow_rotation = allow_rotation
        self.second_barcode_confirmed = second_barcode_confirmed

    def render(self, job: dict) -> Mapping[str, bytes]:
        _validate_job(job)
        try:
            from openpyxl import load_workbook
        except ImportError as exc:
            raise ImportError("JafWebCutAdapter requires optional openpyxl>=3.1,<4") from exc
        workbook = load_workbook(Path(__file__).parent / "templates" / self.filename)
        try:
            sheet, export = workbook["všeobecný"], workbook["Export"]
            capacity = 0
            for row in range(18, sheet.max_row + 1):
                if (sheet.cell(row, 3).data_type != "f"
                        or export.cell(row - 16, 15).data_type != "f"):
                    break
                capacity += 1
            if len(job["parts"]) > capacity:
                raise ManufacturingError(f"JAF template row capacity exceeded ({capacity})")
            for row in sheet.iter_rows(min_row=18, min_col=4, max_col=21):
                for cell in row:
                    cell.value = None
            # Remove stale formula results and phantom unused rows from the export grid.
            for row in export.iter_rows(min_row=2, max_col=16):
                for cell in row:
                    cell.value = None
            group, position, last_group = 0, 0, None
            for offset, part in enumerate(job["parts"]):
                code = part["code"]
                setups = {setup["id"]: setup["code"] for setup in part["machining_setups"]}
                if not set(setups) <= {"A", "B"} or ("B" in setups and "A" not in setups):
                    raise ManufacturingError("JAF supports only setup A or A/B")
                if "B" in setups and not self.second_barcode_confirmed:
                    raise ManufacturingError("JAF second barcode U/P requires explicit destination confirmation")
                if part.get("stock_shape") != "rectangular_prism":
                    raise ManufacturingError("JAF requires explicit rectangular stock; profile bounds are not a cut piece")
                if any(len(value) > 40 for value in [code, *setups.values()]):
                    raise ManufacturingError("JAF template ID/barcode accepts at most 40 characters")
                material = self.material_mapping.get(part["material_key"])
                if material is None:
                    raise ManufacturingError(f"missing JAF material mapping: {part['material_key']}")
                length, width, thickness = part["dimensions_mm"]
                key = material, thickness
                if key != last_group:
                    group, position, last_group = group + 1, 0, key
                position += 1
                r = 18 + offset
                values = {4: group, 5: str(position), 6: material, 7: thickness,
                          8: part["name"], 9: 1, 10: length, 11: width,
                          12: None if self.allow_rotation else "x", 19: code,
                          20: setups.get("A"), 21: setups.get("B")}
                for column, value in values.items():
                    self._cell(sheet.cell(r, column), value)
                # Exact equivalents of the original Export A..P formulas for these inputs.
                combined = material + " " + format(thickness, ".15g")
                out = [group, str(position), combined, part["name"], 1, length, width,
                       int(self.allow_rotation), None, None, None, None, None,
                       code, setups.get("A"), setups.get("B")]
                for column, value in enumerate(out, 1):
                    self._cell(export.cell(offset + 2, column), value)
            buffer = io.BytesIO()
            workbook.save(buffer)
            return {self.filename: buffer.getvalue()}
        finally:
            workbook.close()

    @staticmethod
    def _cell(cell, value):
        if isinstance(value, str):
            if len(value) > 32767 or any(ord(c) < 32 and c not in "\t\r\n" for c in value):
                raise ManufacturingError("JAF cell text contains illegal characters or is too long")
            cell.value = value
            cell.data_type = "s"  # Never interpret untrusted names/materials as formulas.
            cell.number_format = "@"
        else:
            cell.value = value
