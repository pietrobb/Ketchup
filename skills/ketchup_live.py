"""Explicit new-window live bridge tools. Never owns or terminates the GUI."""
from __future__ import annotations

import asyncio
import importlib.util
import json
import os
from pathlib import Path
import secrets
import select
import subprocess
import threading
import time
import uuid

from anthropic.lib.tools import beta_async_tool

MAX_OUTPUT = 32768
MAX_STARTUP = 1024
MAX_SESSIONS = 4
IMAGE_ROOT = Path(__file__).resolve().parents[1] / "artifacts" / "live-view"

# Reuse the offline loader/plan binding only. Do not register its tools, create
# its Runtime, alter sys.path, or import/shadow the public ketchup package.
_spec = importlib.util.spec_from_file_location(
    "_supervisor_ketchup_live_helpers", Path(__file__).with_name("ketchup_model.py"))
_helpers = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_helpers)
_sdk = _helpers._sdk
_plan_state = _helpers._plan_state


def _live():
    return _sdk().live


class Rejection(ValueError):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code


def _error(code, message, unknown=False):
    return {"ok": False, "error": {"code": code, "message": message},
            "mutation_outcome_unknown": unknown, "retry_mutation": False}


def _output(value):
    text = json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(",", ":"))
    if len(text.encode("utf-8")) <= MAX_OUTPUT:
        return text
    fallback = _error("output_too_large", "Result exceeds 32 KiB. Narrow reads; never retry a mutation.")
    fallback.update(complete=False, operation_completed=value.get("ok", False), stamp=value.get("stamp"))
    text = json.dumps(fallback, ensure_ascii=False, separators=(",", ":"))
    if len(text.encode("utf-8")) > MAX_OUTPUT:
        fallback["stamp"] = None
        text = json.dumps(fallback, separators=(",", ":"))
    return text


def _action(action, choices):
    if action not in choices:
        raise Rejection("invalid_action", "Choose one of: " + ", ".join(choices))


def _path(value):
    if type(value) is not str or not value or len(value) > 4096 or "\0" in value:
        raise Rejection("invalid_path", "Supply an explicit absolute existing file path.")
    path = Path(value)
    if not path.is_absolute() or not path.is_file():
        raise Rejection("invalid_path", "Supply an explicit absolute existing file path.")
    return str(path.resolve())


def _pipe_chunk(stream, limit):
    """Read only already available bytes, including on Python 3.11 Windows."""
    if os.name == "nt":
        import ctypes
        import msvcrt
        from ctypes import wintypes
        peek = ctypes.WinDLL("kernel32", use_last_error=True).PeekNamedPipe
        peek.argtypes = [wintypes.HANDLE, wintypes.LPVOID, wintypes.DWORD,
                         wintypes.LPVOID, ctypes.POINTER(wintypes.DWORD), wintypes.LPVOID]
        peek.restype = wintypes.BOOL
        available = wintypes.DWORD()
        if not peek(msvcrt.get_osfhandle(stream.fileno()), None, 0, None,
                    ctypes.byref(available), None):
            if ctypes.get_last_error() in (109, 232):
                return b""
            raise OSError("pipe unavailable")
        if not available.value:
            return None
        return stream.read(min(limit, available.value))
    if not select.select([stream], [], [], 0)[0]:
        return None
    return os.read(stream.fileno(), limit)


def _startup(stream, deadline):
    data = bytearray()
    while time.monotonic() < deadline:
        chunk = _pipe_chunk(stream, min(256, MAX_STARTUP + 1 - len(data)))
        if chunk is None:
            time.sleep(0.01)
            continue
        if not chunk:
            break
        data.extend(chunk)
        if b"\n" in data:
            line = bytes(data).split(b"\n", 1)[0]
            if len(line) > MAX_STARTUP:
                break
            value = json.loads(line, object_pairs_hook=_live()._object)
            if (type(value) is not dict or set(value) != {"version", "live_bridge_address"}
                    or type(value["version"]) is not int or value["version"] != 1):
                break
            # Do not pass arbitrary startup strings to even a trusted factory.
            address = value["live_bridge_address"]
            if type(address) is not str or not address.startswith("127.0.0.1:"):
                break
            port = address[10:]
            if (not port.isascii() or not port.isdecimal() or not 1 <= len(port) <= 5
                    or not 1 <= int(port) <= 65535 or str(int(port)) != port):
                break
            return address
        if len(data) > MAX_STARTUP:
            break
    raise Rejection("launch_failed", "Live GUI startup failed or timed out; the window may remain open.")


class _Drain:
    """Bounded-memory discard of ongoing stdout; no blocking reads or logging."""
    def __init__(self, stream):
        self.stream = stream
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True, name="ketchup-live-stdout")
        self.thread.start()

    def run(self):
        try:
            while not self.stop.is_set():
                chunk = _pipe_chunk(self.stream, 4096)
                if chunk == b"":
                    break
                self.stop.wait(0.01)
        except Exception:
            pass
        finally:
            try:
                self.stream.close()
            except Exception:
                pass

    def close(self):
        self.stop.set()
        self.thread.join(timeout=0.5)


class _LaunchedSession:
    def __init__(self, session, drain, process):
        self._session, self._drain, self._process = session, drain, process

    def __getattr__(self, name):
        return getattr(self._session, name)

    def close(self):
        try:
            self._session.close()
        finally:
            self._drain.close()
            # No wait/kill/terminate: the GUI and unsaved work belong to the user.
            self._process = None


def _launch(executable, document_path=None, *, session_factory=None, timeout=10.0):
    """Trusted host-only factory seam; credentials never enter tool arguments."""
    process = drain = session = None
    if type(timeout) not in (int, float) or not 0 < timeout <= 30:
        raise Rejection("invalid_arguments", "Startup timeout must be in (0, 30] seconds.")
    executable = _path(executable)
    document_path = _path(document_path) if document_path else None
    deadline = time.monotonic() + timeout
    try:
        token = secrets.token_hex(32)
        command = [executable, "--supervisor-live-stdin"]
        if document_path:
            command.append(document_path)
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.DEVNULL, bufsize=0, shell=False)
        bootstrap = json.dumps({"version": 1, "token": token}, separators=(",", ":")).encode("ascii") + b"\n"
        if len(bootstrap) > MAX_STARTUP or process.stdin.write(bootstrap) != len(bootstrap):
            raise ValueError("bootstrap unavailable")
        process.stdin.close()
        address = _startup(process.stdout, deadline)
        drain = _Drain(process.stdout)
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError()
        factory = session_factory or _sdk().LiveSession
        session = factory(address, token, timeout=min(remaining, 30.0))
        return _LaunchedSession(session, drain, process)
    except BaseException:
        if session is not None:
            try:
                session.close()
            except Exception:
                pass
        if drain is not None:
            drain.close()
        if process is not None:
            for stream in (process.stdin, process.stdout if drain is None else None):
                if stream is not None:
                    try:
                        stream.close()
                    except Exception:
                        pass
        # Never expose subprocess/JSON/socket exception text or chained secrets.
        raise Rejection("launch_failed", "Live GUI startup failed or timed out; the window may remain open.") from None


class Runtime:
    def __init__(self, plan_state, launcher=None):
        self.plan_state = plan_state
        self.launcher, self.sdk = launcher or _launch, _live()
        self.sessions = {}
        self.lock = asyncio.Lock()

    def guard(self):
        if self.plan_state is None or type(getattr(self.plan_state, "active", None)) is not bool:
            raise Rejection("plan_guard_unavailable", "No Supervisor plan state binding; launch and mutations disabled.")
        if self.plan_state.active:
            raise Rejection("plan_mode", "Live launch and mutations are forbidden in Supervisor plan mode.")

    def entry(self, handle):
        try:
            if str(uuid.UUID(handle)) != handle:
                raise ValueError()
            return self.sessions[handle]
        except (ValueError, TypeError, KeyError, AttributeError):
            raise Rejection("invalid_handle", "Unknown or disconnected live session UUID.") from None

    def forget(self, handle):
        session = self.sessions.pop(handle, None)
        if session is not None:
            try:
                session.close()
            except Exception:
                pass

    def expected(self, value):
        result = _live()._stamp(value)
        if not result["canonical_digest"]:
            raise Rejection("precondition_required", "Supply a complete caller-observed stamp, including digest and mutation_epoch.")
        return result

    def edit_preflight(self, session, expected, selection):
        selected = _live()._ids(selection)
        fresh = session.status()
        if fresh["stamp"] != expected:
            raise Rejection("stale_document", "Caller stamp is stale; inspect before preparing another edit.")
        if fresh["result"].get("selection") is None or sorted(fresh["result"]["selection"]) != sorted(selected):
            raise Rejection("selection_changed", "Caller selection differs from the GUI selection.")
        # Observation is compared, NEVER substituted for the caller's stamp.
        self.guard()
        return selected

    async def run(self, handle, job, *, mutation=False):
        async with self.lock:
            task = asyncio.create_task(asyncio.to_thread(job))
            try:
                return _output(await asyncio.shield(task))
            except asyncio.CancelledError:
                # Keep serialization until the bounded job settles, then dispose
                # only our socket/pipes, including a just-launched session.
                while not task.done():
                    try:
                        await asyncio.shield(task)
                    except asyncio.CancelledError:
                        continue
                    except Exception:
                        break
                if task.done() and not task.cancelled():
                    task.exception()
                self.forget(handle)
                raise
            except Exception as error:
                if isinstance(error, Rejection):
                    value = _error(error.code, str(error))
                elif isinstance(error, self.sdk.LiveBridgeError):
                    code = error.code if error.code in self.sdk._ERROR_CODES else "remote_error"
                    value = _error(code, "Live bridge rejected the request.")
                elif isinstance(error, self.sdk.LiveTransportError):
                    self.forget(handle)
                    value = _error("live_transport_error", "Live connection failed. Do not retry mutations.",
                                   bool(error.mutation_outcome_unknown))
                elif isinstance(error, (ValueError, TypeError)):
                    value = _error("invalid_arguments", "Invalid live tool arguments; no automatic retry.", mutation)
                else:
                    self.forget(handle)
                    value = _error("live_operation_failed", "Live operation failed; details suppressed. Do not retry mutations.", mutation)
                return _output(value)


def register_tools() -> list:
    return _register_tools(_plan_state())


def _register_tools(plan_state, *, launcher=None) -> list:
    """Python host/test injection only; not exposed in any tool schema."""
    runtime = Runtime(plan_state, launcher)

    @beta_async_tool(name="KetchupLiveSession")
    async def session(action: str, handle: str = "", executable: str = "", document_path: str = "") -> str:
        """Launch and attach a NEW GUI window, never an already-running window. Disconnect never quits the GUI.

        Args:
            action: launch or disconnect. Launch is forbidden in plan mode.
            handle: Live UUID, required only for disconnect (also allowed in plan mode).
            executable: Explicit absolute existing GUI executable, required for launch. No discovery or extra arguments.
            document_path: Optional absolute existing document file for the new window; never replaces an existing window.
        """
        owned = str(uuid.uuid4()) if action == "launch" else handle
        def job():
            _action(action, ("launch", "disconnect"))
            if action == "disconnect":
                if executable or document_path:
                    raise Rejection("invalid_arguments", "Disconnect accepts only a live handle.")
                runtime.entry(handle)
                runtime.forget(handle)
                return {"ok": True, "result": {"disconnected": handle, "app_terminated": False}}
            runtime.guard()
            if handle:
                raise Rejection("invalid_arguments", "Launch creates a new handle and new GUI window.")
            if len(runtime.sessions) >= MAX_SESSIONS:
                raise Rejection("session_limit", "Disconnect a live session first; maximum is four.")
            binary = _path(executable)
            path = _path(document_path) if document_path else None
            runtime.guard()
            live_session = runtime.launcher(binary, path)
            runtime.sessions[owned] = live_session
            try:
                runtime.guard()
                result = live_session.status()
                return {**result, "result": {**result["result"], "handle": owned,
                        "ownership": "nonowning_new_GUI_window", "plan_guard_bound": True,
                        "attachment": "This is the newly launched window, not any already-running window."}}
            except BaseException:
                runtime.forget(owned)
                raise
        return await runtime.run(owned, job)

    @beta_async_tool(name="KetchupLiveInspect")
    async def inspect(action: str, handle: str, expected: dict | None = None,
                      kind: str = "occurrences", entity_id: int = 0, limit: int = 50,
                      search: str = "", definition_id: int | None = None, tag_id: int | None = None,
                      classification_dimension_id: int | None = None,
                      classification_category_id: int | None = None, cursor: str | None = None,
                      world_bounds_mm: list[list[float]] | None = None,
                      workset_handle: str = "") -> str:
        """Read live status/summary/query/detail. Results retain the complete bridge stamp; no geometry is fabricated.

        Args:
            action: status, summary, query, detail, workset_create, or workset_status. Allowed in plan mode.
            handle: Live session UUID.
            expected: Complete observed stamp required for query/detail: document_id, revision, canonical_digest, mutation_epoch.
            kind: occurrences, instances, definitions, features, or relations. Relations stream canonical hierarchy, definition-use, and assembly edges.
            entity_id: Positive ID for detail; not valid for instances or relations.
            limit: Query page size 1 through 100.
            search: Name substring or relation type (uses_definition/member_of_group/assembly_mate), at most 128 UTF-8 bytes.
            definition_id: Optional positive definition filter for occurrence/instance/feature/relation queries.
            tag_id: Optional exact tag filter for occurrence/instance queries; instances match any path step.
            classification_dimension_id: Optional root-occurrence classification dimension filter.
            classification_category_id: Optional category filter; requires classification_dimension_id.
            cursor: Optional opaque query continuation, never refreshed automatically.
            world_bounds_mm: Optional inclusive [[min x,y,z],[max x,y,z]] world AABB for instance query/workset.
            workset_handle: Opaque handle required only for workset_status.
        """
        def job():
            _action(action, ("status", "summary", "query", "detail", "workset_create", "workset_status"))
            if action == "workset_create" and cursor:
                raise Rejection("invalid_arguments", "workset_create requires a complete query scope without cursor.")
            if action == "workset_status":
                if type(workset_handle) is not str or not workset_handle or len(workset_handle) > 4096:
                    raise Rejection("invalid_arguments", "workset_status requires a bounded opaque handle.")
            elif workset_handle:
                raise Rejection("invalid_arguments", "workset_handle applies only to workset_status.")
            live_session = runtime.entry(handle)
            if action in ("status", "summary"):
                return getattr(live_session, action)()
            stamp = runtime.expected(expected)
            if action == "detail":
                return live_session.detail(stamp, kind, entity_id)
            if action == "workset_status":
                return live_session.workset_status(stamp, workset_handle)
            if action == "workset_create":
                return live_session.create_workset(
                    stamp, kind=kind, limit=limit, search=search,
                    definition_id=definition_id, tag_id=tag_id,
                    classification_dimension_id=classification_dimension_id,
                    classification_category_id=classification_category_id,
                    world_bounds_mm=world_bounds_mm)
            return live_session.query(stamp, kind=kind, limit=limit, search=search,
                                      definition_id=definition_id, tag_id=tag_id,
                                      classification_dimension_id=classification_dimension_id,
                                      classification_category_id=classification_category_id,
                                      cursor=cursor, world_bounds_mm=world_bounds_mm)
        return await runtime.run(handle, job)

    @beta_async_tool(name="KetchupLiveEdit")
    async def edit(action: str, handle: str, expected: dict, selection: list[int],
                   program: dict | None = None, proposal_id: int = 0) -> str:
        """Propose/commit/undo/redo with explicit complete caller stamp and GUI selection. Never refresh or retry mutations.

        Args:
            action: propose, commit, undo, or redo; all forbidden in plan mode.
            handle: Live session UUID.
            expected: Exact observed document_id, revision, canonical_digest, mutation_epoch; all four required.
            selection: Explicit observed root occurrence IDs, including [] if empty. Preflight checks selection; commit also checks its proposal selection atomically in Rust.
            program: For propose only, typed CAD program object containing operations; Rust validates and plans it.
            proposal_id: For commit only, positive proposal ID returned on this connection. No automatic commit/retry.
        """
        def job():
            _action(action, ("propose", "commit", "undo", "redo"))
            runtime.guard()
            live_session = runtime.entry(handle)
            stamp = runtime.expected(expected)
            selected = runtime.edit_preflight(live_session, stamp, selection)
            if action == "propose":
                return live_session.propose(stamp, selected, program)
            if action == "commit":
                return live_session.commit(stamp, proposal_id)
            return getattr(live_session, action)(stamp)
        return await runtime.run(handle, job, mutation=True)

    @beta_async_tool(name="KetchupLiveView")
    async def view(action: str, handle: str, expected: dict,
                   occurrence_ids: list[int] | None = None, view: str = "", image_path: str = "",
                   capture_mode: str = "offscreen") -> str:
        """Guarded live view commands and CAD PNG artifacts; never desktop screenshots.

        Saving an artifact does not establish visual delivery or geometry correctness.

        Args:
            action: selection, view, or image. All are forbidden in plan mode (image writes a file).
            handle: Live session UUID.
            expected: Complete caller-observed document_id, revision, canonical_digest, mutation_epoch.
            occurrence_ids: Explicit root occurrence IDs for selection, including [] to clear.
            view: For view action, iso, top, front, or zoom_fit.
            image_path: Required only for image: explicit absolute NEW .png under workspace artifacts/live-view; never overwritten.
            capture_mode: For image, offscreen (default AI render) or visible_viewport (proof of a visible focused canvas).
        """
        def job():
            _action(action, ("selection", "view", "image"))
            runtime.guard()
            if action != "image" and (image_path or capture_mode != "offscreen"):
                raise Rejection("invalid_arguments", "image_path and capture_mode apply only to image.")
            live_session = runtime.entry(handle)
            stamp = runtime.expected(expected)
            if action == "image":
                if occurrence_ids is not None or view:
                    raise Rejection("invalid_arguments", "Image accepts only expected stamp and image_path.")
                try:
                    destination = runtime.sdk._image_path(image_path)
                    destination.relative_to(IMAGE_ROOT)
                except FileExistsError:
                    raise Rejection("file_exists", "Image destination exists; choose a NEW .png path.") from None
                except (ValueError, TypeError, OSError):
                    raise Rejection("invalid_path", "Supply an absolute NEW .png under workspace artifacts/live-view, without links.") from None
                response = live_session.image(stamp, capture_mode=capture_mode)
                runtime.guard()  # Plan may have changed while the bounded read ran.
                try:
                    return runtime.sdk.save_image(
                        response, stamp, str(destination), capture_mode=capture_mode
                    )
                except FileExistsError:
                    raise Rejection("file_exists", "Image destination exists; choose a NEW .png path.") from None
            runtime.guard()
            if action == "selection":
                return live_session.selection(stamp, occurrence_ids)
            return live_session.view(stamp, view)
        return await runtime.run(handle, job, mutation=action in ("selection", "view"))

    return [session, inspect, edit, view]
