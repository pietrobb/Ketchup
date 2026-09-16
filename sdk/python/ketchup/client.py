"""Bounded synchronous JSON-lines client, with cross-platform pipe readers.

No shell, embedded Python, GUI, sockets, or assistant sidecar is involved.
Transport failures close the session: a timed-out mutation has unknown outcome
and MUST NOT be retried automatically. Reopen a saved document to recover.
"""
from __future__ import annotations

import copy
import json
import math
import os
from pathlib import Path
import queue
import subprocess
import threading
from collections.abc import Mapping
from typing import Any

PROTOCOL = "ketchup.headless.v1"
MAX_LINE_BYTES = 4 * 1024 * 1024
MAX_STDERR_BYTES = 64 * 1024
MAX_TIMEOUT_MS = 300_000


class HeadlessError(RuntimeError):
    """Server rejection; code and diagnostic details are preserved unchanged."""
    def __init__(self, code: str, message: str, details: Any = None):
        super().__init__(message)
        self.code, self.message, self.details = code, message, details


class TransportError(RuntimeError):
    pass


class ProtocolError(TransportError):
    pass


class TransportTimeout(TransportError):
    pass


class SessionClosedError(TransportError):
    pass


def _resolve(value: str | os.PathLike | None, env: Mapping[str, str],
             variable: str, name: str) -> str:
    value = os.fspath(value) if value is not None else env.get(variable, name)
    path = Path(value).expanduser()
    extensions = [""]
    if os.name == "nt" and not path.suffix:
        extensions = [extension for extension in env.get("PATHEXT", ".COM;.EXE;.BAT;.CMD").split(os.pathsep)
                      if extension]

    if path.is_absolute() or os.path.dirname(value):
        candidates = [Path(f"{path}{extension}") for extension in extensions]
    else:
        current_directory = Path.cwd().resolve()
        candidates = []
        for directory_value in env.get("PATH", "").split(os.pathsep):
            directory = Path(directory_value).expanduser()
            if not directory.is_absolute() or directory.resolve() == current_directory:
                continue
            candidates.extend(Path(f"{directory / path}{extension}") for extension in extensions)

    for candidate in candidates:
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate.resolve())
    raise FileNotFoundError(f"Cannot locate {name}: {value!r}; supply a path or {variable}")


def _no_constant(value: str) -> None:
    raise ValueError(f"nonfinite JSON number: {value}")


def _object(pairs: list) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON field: {key}")
        result[key] = value
    return result


def _finite(value: Any) -> None:
    if isinstance(value, float) and not math.isfinite(value):
        raise ValueError("nonfinite JSON number")
    if isinstance(value, dict):
        for item in value.values():
            _finite(item)
    elif isinstance(value, list):
        for item in value:
            _finite(item)


def _is_u64(value: Any) -> bool:
    return type(value) is int and 0 <= value <= (1 << 64) - 1


def _is_canonical_digest(value: Any) -> bool:
    return (isinstance(value, str) and len(value) == 16
            and all(character in "0123456789abcdef" for character in value))


class Session:
    """Own one ketchup-headless process. Use as a context manager.

    executable/worker override KETCHUP_HEADLESS/KETCHUP_EXACT_WORKER and PATH.
    Without a worker override, the CLI uses its sibling ketchup-exact-worker.
    env is merged with the current environment (for OCCT DLL search paths).
    A session serializes requests. Document handles expire on new/open/close,
    including possibly-applied new/open errors. After those errors, explicitly
    call new_document/open_document again; fresh summary guards and the normal
    discard_unsaved check protect any remaining work. No automatic retry occurs.
    """
    def __init__(self, executable=None, worker=None, *, timeout=30.0, env=None, compact=False):
        if isinstance(timeout, bool) or not isinstance(timeout, (int, float)) or not math.isfinite(timeout) or not 0 < timeout <= 600:
            raise ValueError("timeout must be finite and in (0, 600] seconds")
        self.timeout = float(timeout)
        self._lock = threading.RLock()
        self._stderr_lock = threading.Lock()
        self._stderr = bytearray()
        self._stop = threading.Event()
        self._responses = queue.Queue(maxsize=2)
        self._closed = False
        self._generation = 0
        self._id = 0
        self._state, self.compact = None, compact
        process_env = dict(os.environ)
        if env:
            process_env.update(env)
        argv = [_resolve(executable, process_env, "KETCHUP_HEADLESS", "ketchup-headless"), "--stdio"]
        if worker is not None or process_env.get("KETCHUP_EXACT_WORKER"):
            argv += ["--worker", _resolve(worker, process_env, "KETCHUP_EXACT_WORKER", "ketchup-exact-worker")]
        self._process = subprocess.Popen(
            argv, shell=False, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, env=process_env, bufsize=0,
        )
        self._threads = [
            threading.Thread(target=self._read_stdout, name="ketchup-stdout", daemon=True),
            threading.Thread(target=self._read_stderr, name="ketchup-stderr", daemon=True),
        ]
        for thread in self._threads:
            thread.start()

    def _publish(self, item):
        while not self._stop.is_set():
            try:
                self._responses.put(item, timeout=0.1)
                return
            except queue.Full:
                continue

    def _read_stdout(self):
        # Use buffered bounded readline even though Popen pipes are unbuffered.
        import io
        try:
            reader = io.BufferedReader(self._process.stdout)
            while not self._stop.is_set():
                line = reader.readline(MAX_LINE_BYTES + 1)
                if not line:
                    self._publish(TransportError("headless stdout reached EOF"))
                    return
                if len(line) > MAX_LINE_BYTES or not line.endswith(b"\n"):
                    self._publish(ProtocolError("oversized or unterminated response"))
                    return
                self._publish(line)
        except (OSError, ValueError) as error:
            self._publish(TransportError(f"stdout read failed: {error}"))

    def _read_stderr(self):
        try:
            while not self._stop.is_set():
                chunk = self._process.stderr.read(4096)
                if not chunk:
                    return
                with self._stderr_lock:
                    self._stderr.extend(chunk)
                    del self._stderr[:-MAX_STDERR_BYTES]
        except (OSError, ValueError):
            return

    @property
    def stderr(self):
        """Bounded diagnostic tail, escaped to prevent terminal control injection."""
        with self._stderr_lock:
            text = bytes(self._stderr).decode("utf-8", errors="replace")
        return "".join(c if c.isprintable() or c == "\n" else f"\\u{ord(c):04x}" for c in text)

    def _ensure_open(self):
        if self._closed:
            raise SessionClosedError("session is closed")

    def _request(self, method, params=None, *, timeout=None):
        with self._lock:
            self._ensure_open()
            self._id += 1
            request_id = self._id
            # Serialize before sending; NaN, Infinity and oversize never reach stdin.
            data = (json.dumps({"protocol": PROTOCOL, "id": request_id, "method": method,
                                "params": params or {}}, allow_nan=False, separators=(",", ":")) + "\n").encode("utf-8")
            if len(data) > MAX_LINE_BYTES:
                raise ValueError("request exceeds maximum line size")
            wait = self.timeout if timeout is None else timeout
            written = queue.Queue(maxsize=1)

            def write():
                try:
                    view = memoryview(data)
                    while view:
                        count = self._process.stdin.write(view)
                        if not count:
                            raise BrokenPipeError("headless stdin closed")
                        view = view[count:]
                    self._process.stdin.flush()
                    written.put(None)
                except (OSError, ValueError) as error:
                    written.put(error)

            # A non-reading child must not hang the caller on a full stdin pipe.
            writer = threading.Thread(target=write, name="ketchup-stdin", daemon=True)
            writer.start()
            import time
            deadline = time.monotonic() + wait
            try:
                error = written.get(timeout=wait)
                if error is not None:
                    raise TransportError(f"stdin write failed: {error}")
                line = self._responses.get(timeout=max(0.001, deadline - time.monotonic()))
                if isinstance(line, Exception):
                    raise line
                try:
                    response = json.loads(line, parse_constant=_no_constant, object_pairs_hook=_object)
                    _finite(response)
                except (ValueError, UnicodeError, RecursionError) as error:
                    raise ProtocolError(f"invalid response JSON: {error}") from error
                if not isinstance(response, dict) or response.get("protocol") != PROTOCOL:
                    raise ProtocolError("invalid response protocol")
                if type(response.get("id")) is not int or response["id"] != request_id:
                    raise ProtocolError("response id mismatch")
                if set(response) == {"protocol", "id", "error"}:
                    error = response["error"]
                    if (not isinstance(error, dict) or not {"code", "message"} <= set(error)
                            or set(error) - {"code", "message", "details"}
                            or not isinstance(error["code"], str) or not isinstance(error["message"], str)):
                        raise ProtocolError("invalid error envelope")
                    raise HeadlessError(error["code"], error["message"], error.get("details"))
                if set(response) != {"protocol", "id", "result"} or not isinstance(response["result"], dict):
                    raise ProtocolError("invalid result envelope")
                return response["result"]
            except queue.Empty as error:
                self.close()
                raise TransportTimeout(f"headless request timed out; outcome unknown; stderr: {self.stderr}") from error
            except TransportError as error:
                self.close()
                raise type(error)(f"{error}; stderr: {self.stderr}") from error
            finally:
                writer.join(timeout=1)

    def capabilities(self):
        return self._request("capabilities")

    def _observe(self, result):
        state = result.get("state")
        if (not isinstance(state, dict)
                or not _is_u64(state.get("document_id"))
                or not _is_u64(state.get("revision"))
                or not _is_canonical_digest(state.get("canonical_digest"))
                or not _is_u64(state.get("mutation_epoch"))):
            self.close()
            raise ProtocolError("missing or invalid normalized state")
        self._state = copy.deepcopy(state)
        return result

    def _expected(self):
        if self._state is None:
            self._observe(self._request("summary"))
        return {"expected_revision": self._state["revision"],
                "expected_digest": self._state["canonical_digest"],
                "expected_mutation_epoch": self._state["mutation_epoch"]}

    def _replace(self, method, *, discard_unsaved=False, **params):
        with self._lock:
            params.update(self._expected(), **({"response": "compact"} if self.compact else {}))
            params["discard_unsaved"] = discard_unsaved
            try:
                self._observe(self._request(method, params))
            except HeadlessError as error:
                if (isinstance(error.details, dict)
                        and error.details.get("mutation_outcome") == "possibly_applied"):
                    # The server may now own a different document. Never rebind old handles
                    # or terminate potentially unsaved work; require explicit new/open.
                    self._generation += 1
                    self._state = None
                raise
            self._generation += 1
            return Document(self, self._generation)

    def new_document(self, *, discard_unsaved=False):
        return self._replace("new", discard_unsaved=discard_unsaved)

    def open_document(self, path, *, discard_unsaved=False):
        return self._replace("open", path=os.fspath(path), discard_unsaved=discard_unsaved)

    def close(self):
        with self._lock:
            if self._closed:
                return
            self._closed = True
            self._generation += 1
            self._stop.set()
            # Terminate first: closing a pipe while another thread writes can block.
            if self._process.poll() is None:
                self._process.terminate()
            try:
                self._process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self._process.kill()
                self._process.wait(timeout=2)
            for pipe in (self._process.stdin, self._process.stdout, self._process.stderr):
                try:
                    pipe.close()
                except (OSError, ValueError):
                    pass
            for thread in self._threads:
                if thread is not threading.current_thread():
                    thread.join(timeout=1)

    def __enter__(self):
        self._ensure_open()
        return self

    def __exit__(self, *_):
        self.close()


class Document:
    """Revision-bound handle. Successful mutations refresh observed state.

    `apply` accepts a full program mapping or a list of typed operation mappings.
    All operations in that call form ONE atomic CAD program/Undo step. IDs of
    newly allocated objects cannot be referenced in that same program: use a
    subsequent call with IDs in result['created']. No whole-script atomicity.
    """
    def __init__(self, session, generation):
        self._session, self._generation = session, generation
        self.validators = Validators(self)

    def _check(self):
        self._session._ensure_open()
        if self._generation != self._session._generation:
            raise SessionClosedError("document handle expired after new/open")

    def _call(self, method, params=None, *, mutation=False, guarded=False, state=False, timeout=None):
        with self._session._lock:
            self._check()
            params = dict(params or {})
            if mutation or guarded:
                params.update(self._session._expected())
            if mutation:
                params.update({"response": "compact"} if self._session.compact else {})
            result = self._session._request(method, params, timeout=timeout)
            if state:
                self._session._observe(result)
            return result

    def detail(self, kind, id): return self._call("detail", {"kind": kind, "id": id})
    @property
    def state(self):
        return self._call("state", state=True)["state"]
    def summary(self): return self._call("summary", state=True)
    def apply(self, operations, *, selection=()):
        program = dict(operations) if isinstance(operations, Mapping) else {"operations": list(operations)}
        return self._call("apply", {"program": program, "selection": list(selection)}, mutation=True, state=True)
    def query(self, **params): return self._call("query", params)
    def create_workset(self, **params): return self._call("workset_create", params)
    def workset_status(self, handle): return self._call("workset_status", {"handle": handle})
    def start_batch_job(self, workset_handle, operation):
        return self._call("batch_job_start", {
            "workset_handle": workset_handle, "operation": dict(operation)}, guarded=True)
    def batch_job_status(self, handle):
        return self._call("batch_job_status", {"handle": handle})
    def cancel_batch_job(self, handle):
        return self._call("batch_job_cancel", {"handle": handle})
    def step_batch_job(self, handle):
        with self._session._lock:
            result = self._call("batch_job_step", {"handle": handle}, guarded=True)
            receipt = result.get("receipt")
            if isinstance(receipt, dict) and isinstance(receipt.get("after"), dict):
                self._session._state.update(receipt["after"])
            return result
    def create_part(self, name, entities, *, feature, constraints=(), plane="xy",
                    translation_mm=(0, 0, 0), rotation=None):
        operation = {"operation": "create_part", "name": name,
                     "workplane": {"type": "principal", "plane": plane},
                     "entities": list(entities), "constraints": list(constraints),
                     "feature": feature, "translation_mm": list(translation_mm)}
        if rotation is not None:
            operation["rotation"] = rotation
        return self.apply([operation])

    def extrude(self, name, profile, distance_mm, **kwargs):
        return self.create_part(name, profile, feature={"type": "extrusion", "distance_mm": distance_mm}, **kwargs)

    def box(self, name, width_mm, depth_mm, height_mm, **kwargs):
        """Rectangle sketch + universal extrusion, not a special box primitive."""
        return self.extrude(name, rectangle(width_mm, depth_mm), height_mm, **kwargs)

    def create_sketch(self, definition_id, name, entities, *, constraints=(), workplane=None):
        return self.apply([{"operation": "create_sketch", "definition_id": definition_id,
                            "name": name, "entities": list(entities), "constraints": list(constraints),
                            "workplane": workplane or {"type": "principal", "plane": "xy"}}])

    def append_feature(self, definition_id, name, feature):
        return self.apply([{"operation": "append_feature", "definition_id": definition_id,
                            "name": name, "feature": feature}])

    def pocket(self, definition_id, name, target_feature_id, profile_feature_id, depth_mm):
        return self.append_feature(definition_id, name, {"type": "pocket", "target_feature_id": target_feature_id,
                                   "profile_feature_id": profile_feature_id, "depth_mm": depth_mm})

    def planar_surface(self, definition_id, name, profile_feature_id):
        return self.append_feature(definition_id, name, {
            "type": "surface_body",
            "source": {"type": "planar", "profile_feature_id": profile_feature_id},
        })

    def loft_surface(self, definition_id, name, sections, *, guide_feature_id=None,
                     continuity="position"):
        source = {"type": "loft", "sections": list(sections), "continuity": continuity}
        if guide_feature_id is not None:
            source["guide_feature_id"] = guide_feature_id
        return self.append_feature(definition_id, name, {"type": "surface_body", "source": source})

    def trim_surface(self, definition_id, name, target_feature_id, cutter_feature_id):
        return self.append_feature(definition_id, name, {
            "type": "surface_trim", "target_feature_id": target_feature_id,
            "cutter_feature_id": cutter_feature_id,
        })

    def extend_surface(self, definition_id, name, target_feature_id, distance_mm):
        return self.append_feature(definition_id, name, {
            "type": "surface_extend", "target_feature_id": target_feature_id,
            "distance_mm": distance_mm,
        })

    def knit_surfaces(self, definition_id, name, surface_feature_ids, *, tolerance_mm,
                      make_solid=False):
        return self.append_feature(definition_id, name, {
            "type": "surface_knit", "surface_feature_ids": list(surface_feature_ids),
            "tolerance_mm": tolerance_mm, "make_solid": make_solid,
        })

    def thicken_surface(self, definition_id, name, target_feature_id, thickness_mm,
                        *, direction="inward"):
        return self.append_feature(definition_id, name, {
            "type": "surface_thicken", "target_feature_id": target_feature_id,
            "thickness_mm": thickness_mm, "direction": direction,
        })

    def cam_setup(self, plan_id, name, target_definition_id, target_feature_id, *,
                  stock_minimum_mm, stock_maximum_mm, tool_number, tool_kind,
                  tool_diameter_mm, flute_length_mm, overall_length_mm,
                  holder_diameter_mm, holder_length_mm, spindle_rpm,
                  feed_mm_per_min, plunge_mm_per_min, work_offset="g54",
                  origin_mm=(0, 0, 0), x_axis=(1, 0, 0), y_axis=(0, 1, 0),
                  safe_height_mm, maximum_stepdown_mm, stepover_ratio,
                  radial_allowance_mm=0, axial_allowance_mm=0):
        """Commit one canonical, exact-target-bound CAM setup as one Undo step."""
        return self.apply([{
            "operation": "upsert_cam_plan", "plan_id": plan_id, "name": name,
            "target_definition_id": target_definition_id,
            "target_feature_id": target_feature_id,
            "stock_minimum_mm": list(stock_minimum_mm),
            "stock_maximum_mm": list(stock_maximum_mm),
            "tool_number": tool_number, "tool_kind": tool_kind,
            "tool_diameter_mm": tool_diameter_mm, "flute_length_mm": flute_length_mm,
            "overall_length_mm": overall_length_mm,
            "holder_diameter_mm": holder_diameter_mm,
            "holder_length_mm": holder_length_mm, "spindle_rpm": spindle_rpm,
            "feed_mm_per_min": feed_mm_per_min,
            "plunge_mm_per_min": plunge_mm_per_min, "work_offset": work_offset,
            "origin_mm": list(origin_mm), "x_axis": list(x_axis), "y_axis": list(y_axis),
            "safe_height_mm": safe_height_mm,
            "maximum_stepdown_mm": maximum_stepdown_mm,
            "stepover_ratio": stepover_ratio,
            "radial_allowance_mm": radial_allowance_mm,
            "axial_allowance_mm": axial_allowance_mm,
        }])

    def cam_preview(self, plan_id, operations, *, fixtures=(),
                    dialect="iso_metric_gcode"):
        """Plan, exact-simulate and review a current CAM program without mutation."""
        if type(plan_id) is not int or plan_id <= 0:
            raise ValueError("plan_id must be a positive integer")
        if not isinstance(operations, (list, tuple)) or not operations:
            raise ValueError("operations must be a nonempty list or tuple")
        if not isinstance(fixtures, (list, tuple)):
            raise ValueError("fixtures must be a list or tuple")
        if dialect not in {"iso_metric_gcode", "controller_neutral_json"}:
            raise ValueError("unsupported CAM postprocessor dialect")
        return self._call("cam_preview", {
            "plan_id": plan_id,
            "operations": [dict(operation) for operation in operations],
            "fixtures": [dict(fixture) for fixture in fixtures],
            "dialect": dialect,
        }, guarded=True)

    def cam_export(self, review_token, path, *, confirmed=False):
        """Revalidate and export one reviewed CAM artifact without replacing files."""
        if not isinstance(review_token, str) or not review_token or len(review_token) > 128:
            raise ValueError("review_token must be a nonempty string of at most 128 characters")
        if type(confirmed) is not bool:
            raise ValueError("confirmed must be boolean")
        return self._call("cam_export", {
            "review_token": review_token,
            "path": os.fspath(path),
            "confirmed": confirmed,
        }, guarded=True)

    def fea_review(self, definition_id, feature_id, occurrence_id, case_id, *, material,
                   constrained_face_ordinals, face_tractions, mesh_levels,
                   maximum_nodes=256, relative_pivot_tolerance=1.0e-12,
                   maximum_small_deformation_ratio=0.05,
                   convergence_relative_tolerance=0.02, confirmed=False):
        """Run a guarded exact-mesh linear-static convergence review without mutation."""
        for name, value in (("definition_id", definition_id), ("feature_id", feature_id),
                            ("occurrence_id", occurrence_id)):
            if type(value) is not int or value <= 0:
                raise ValueError(f"{name} must be a positive integer")
        if not isinstance(case_id, str) or not case_id.strip() or len(case_id) > 128:
            raise ValueError("case_id must be a nonempty string of at most 128 characters")
        if not isinstance(material, dict):
            raise ValueError("material must be a dict")
        if not isinstance(constrained_face_ordinals, (list, tuple)) or not constrained_face_ordinals:
            raise ValueError("constrained_face_ordinals must be a nonempty list or tuple")
        if not isinstance(face_tractions, (list, tuple)) or not face_tractions:
            raise ValueError("face_tractions must be a nonempty list or tuple")
        if not isinstance(mesh_levels, (list, tuple)) or not 2 <= len(mesh_levels) <= 4:
            raise ValueError("mesh_levels must contain two to four refinement levels")
        if type(confirmed) is not bool:
            raise ValueError("confirmed must be boolean")
        return self._call("fea_review", {
            "definition_id": definition_id,
            "feature_id": feature_id,
            "occurrence_id": occurrence_id,
            "case_id": case_id,
            "material": dict(material),
            "constrained_face_ordinals": list(constrained_face_ordinals),
            "face_tractions": [dict(traction) for traction in face_tractions],
            "mesh_levels": [dict(level) for level in mesh_levels],
            "solve_settings": {
                "maximum_nodes": maximum_nodes,
                "relative_pivot_tolerance": relative_pivot_tolerance,
                "maximum_small_deformation_ratio": maximum_small_deformation_ratio,
                "convergence_relative_tolerance": convergence_relative_tolerance,
            },
            "confirmed": confirmed,
        }, guarded=True)

    def pdm_release(self, repository, *, parent_release_id=None, dependencies=(),
                    actor, created_unix_ms, note="", confirmed=False):
        """Create an immutable local release; disk writes require explicit confirmation."""
        if parent_release_id is not None and (
                not isinstance(parent_release_id, str) or not parent_release_id):
            raise ValueError("parent_release_id must be None or a nonempty string")
        if not isinstance(dependencies, (list, tuple)):
            raise ValueError("dependencies must be a list or tuple")
        if not isinstance(actor, str) or not actor:
            raise ValueError("actor must be a nonempty string")
        if type(created_unix_ms) is not int or created_unix_ms <= 0:
            raise ValueError("created_unix_ms must be a positive integer")
        if not isinstance(note, str):
            raise ValueError("note must be a string")
        if type(confirmed) is not bool:
            raise ValueError("confirmed must be boolean")
        return self._call("pdm_release_create", {
            "repository": os.fspath(repository),
            "parent_release_id": parent_release_id,
            "dependencies": [dict(dependency) for dependency in dependencies],
            "audit": {"actor": actor, "created_unix_ms": created_unix_ms, "note": note},
            "confirmed": confirmed,
        }, guarded=True)

    def pdm_open_release(self, repository, release_id):
        """Verify and review an immutable local release without opening it as the live document."""
        if not isinstance(release_id, str) or not release_id:
            raise ValueError("release_id must be a nonempty string")
        return self._call("pdm_release_open", {
            "repository": os.fspath(repository), "release_id": release_id,
        }, guarded=True)

    def pdm_catalog(self, repository):
        """Return the verified bounded local release catalog."""
        return self._call("pdm_catalog", {"repository": os.fspath(repository)}, guarded=True)

    def pdm_compare(self, repository, left_release_id, right_release_id):
        """Compare two verified releases and return lineage/conflict evidence."""
        for name, value in (("left_release_id", left_release_id),
                            ("right_release_id", right_release_id)):
            if not isinstance(value, str) or not value:
                raise ValueError(f"{name} must be a nonempty string")
        return self._call("pdm_compare", {
            "repository": os.fspath(repository),
            "left_release_id": left_release_id,
            "right_release_id": right_release_id,
        }, guarded=True)

    def move(self, occurrence_ids, translation_mm):
        return self.apply([{"operation": "transform", "selector": _selector(occurrence_ids),
                            "translation_mm": list(translation_mm)}])

    def rotate(self, occurrence_ids, angle_degrees, *, axis=(0, 0, 1), pivot_mm=(0, 0, 0)):
        return self.apply([{"operation": "transform", "selector": _selector(occurrence_ids),
                            "translation_mm": [0, 0, 0], "rotation": {"axis": list(axis),
                            "pivot_mm": list(pivot_mm), "angle_degrees": angle_degrees}}])

    def copy(self, occurrence_ids, translation_mm):
        return self.apply([{"operation": "copy", "selector": _selector(occurrence_ids),
                            "translation_mm": list(translation_mm)}])

    def set_color(self, occurrence_ids, color):
        return self.apply([{"operation": "set_color", "selector": _selector(occurrence_ids),
                            "color": None if color is None else list(color)}])

    def set_grounded(self, occurrence_ids, grounded=True):
        return self._call("set_grounded", {"occurrence_ids": list(occurrence_ids), "grounded": grounded},
                          mutation=True, state=True)

    def evaluate(self, *, timeout_ms=30_000):
        if type(timeout_ms) is not int or not 1 <= timeout_ms <= MAX_TIMEOUT_MS:
            raise ValueError(f"timeout_ms must be in [1, {MAX_TIMEOUT_MS}]")
        return self._call("evaluate", {"timeout_ms": timeout_ms},
                          timeout=max(self._session.timeout, timeout_ms / 1000 + 5))

    def start_verify_job(self, *, scope=None, timeout_ms=30_000):
        if type(timeout_ms) is not int or not 1 <= timeout_ms <= MAX_TIMEOUT_MS:
            raise ValueError(f"timeout_ms must be in [1, {MAX_TIMEOUT_MS}]")
        params = {"timeout_ms": timeout_ms}
        if scope is not None:
            if not isinstance(scope, list) or not 1 <= len(scope) <= 100:
                raise ValueError("scope must be a list of 1..100 producer keys")
            normalized = []
            for key in scope:
                if not isinstance(key, Mapping) or set(key) != {"definition_id", "feature_id"}:
                    raise ValueError("each scope entry requires only definition_id and feature_id")
                if any(type(key[field]) is not int or key[field] <= 0
                       for field in ("definition_id", "feature_id")):
                    raise ValueError("scope IDs must be positive integers")
                normalized.append({field: key[field] for field in ("definition_id", "feature_id")})
            if len({(key["definition_id"], key["feature_id"]) for key in normalized}) != len(normalized):
                raise ValueError("scope contains duplicate producer keys")
            params["scope"] = normalized
        return self._call("verify_job_start", params, guarded=True)

    def verify_job_status(self, handle):
        if not isinstance(handle, str) or not handle or len(handle) > 128:
            raise ValueError("Verify job handle must be a nonempty string of at most 128 characters")
        return self._call("verify_job_status", {"handle": handle})

    def cancel_verify_job(self, handle):
        if not isinstance(handle, str) or not handle or len(handle) > 128:
            raise ValueError("Verify job handle must be a nonempty string of at most 128 characters")
        return self._call("verify_job_cancel", {"handle": handle})

    def save(self, path, *, overwrite=False):
        return self._call("save", {"path": os.fspath(path), "overwrite": overwrite}, mutation=True, state=True)

    def undo(self):
        return self._call("undo", mutation=True, state=True)

    def redo(self):
        return self._call("redo", mutation=True, state=True)


class Validators:
    def __init__(self, document):
        self._document = document

    def list(self):
        return self._document._call("list_validators")

    def run(self, ids):
        return self._document._call("run_validators", {"ids": [ids] if isinstance(ids, str) else list(ids)})


def _selector(ids):
    return {"type": "occurrences", "occurrence_ids": list(ids)}


def rectangle(width_mm, depth_mm, *, origin_mm=(0, 0)):
    """Return four generic sketch lines; IDs here are local sketch entity IDs."""
    x, y = origin_mm
    points = [(x, y), (x + width_mm, y), (x + width_mm, y + depth_mm), (x, y + depth_mm)]
    return [{"type": "line", "id": i + 1, "start_mm": list(points[i]),
             "end_mm": list(points[(i + 1) % 4])} for i in range(4)]
