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
import shutil
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
    resolved = shutil.which(value, path=env.get("PATH"))
    if resolved:
        return str(Path(resolved).resolve())
    path = Path(value).expanduser()
    if path.is_file():
        return str(path.resolve())
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


class Session:
    """Own one ketchup-headless process. Use as a context manager.

    executable/worker override KETCHUP_HEADLESS/KETCHUP_EXACT_WORKER and PATH.
    Without a worker override, the CLI uses its sibling ketchup-exact-worker.
    env is merged with the current environment (for OCCT DLL search paths).
    A session serializes requests. Document handles expire on new/open/close.
    """
    def __init__(self, executable=None, worker=None, *, timeout=30.0, env=None):
        if not isinstance(timeout, (int, float)) or not math.isfinite(timeout) or not 0 < timeout <= 600:
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
        self._state = None
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
        if (not isinstance(state, dict) or not isinstance(state.get("canonical_digest"), str)
                or type(state.get("revision")) is not int or "document_id" not in state):
            self.close()
            raise ProtocolError("missing or invalid normalized state")
        self._state = copy.deepcopy(state)
        return result

    def _expected(self):
        if self._state is None:
            self._observe(self._request("state"))
        return {"expected_revision": self._state["revision"],
                "expected_digest": self._state["canonical_digest"]}

    def _replace(self, method, *, discard_unsaved=False, **params):
        with self._lock:
            params.update(self._expected())
            params["discard_unsaved"] = discard_unsaved
            self._observe(self._request(method, params))
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

    def _call(self, method, params=None, *, mutation=False, state=False, timeout=None):
        with self._session._lock:
            self._check()
            params = dict(params or {})
            if mutation:
                params.update(self._session._expected())
            result = self._session._request(method, params, timeout=timeout)
            if state:
                self._session._observe(result)
            return result

    @property
    def state(self):
        return self._call("state", state=True)["state"]

    def apply(self, operations, *, selection=()):
        program = dict(operations) if isinstance(operations, Mapping) else {"operations": list(operations)}
        return self._call("apply", {"program": program, "selection": list(selection)}, mutation=True, state=True)

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
