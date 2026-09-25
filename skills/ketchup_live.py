"""Discoverable non-owning live bridge tools. Never terminates the GUI."""
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
SESSION_TIMEOUT = 30.0
# Actionable next step for rejections an agent can fix by itself.
_REJECTION_HINTS = {
    "save_path_required": "The window has no bound file (new or crash-recovered document); "
                          "use KetchupLiveFile save_as with an explicit absolute path.",
    "invalid_params": "The request does not match the protocol; fix the arguments using details.reason.",
    "stale_document": "The document changed since that stamp; re-read status and retry with the new stamp.",
}
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


def _error(code, message, unknown=False, details=None):
    value = {"ok": False, "error": {"code": code, "message": message},
             "mutation_outcome_unknown": unknown, "retry_mutation": False}
    if details is not None:
        value["details"] = details
    return value


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


def _destination(value):
    if type(value) is not str or not value or len(value) > 4096 or "\0" in value:
        raise Rejection("invalid_path", "Supply an explicit absolute destination path.")
    path = Path(value)
    if not path.is_absolute():
        raise Rejection("invalid_path", "Supply an explicit absolute destination path.")
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
    """A window this runtime launched. It keeps the launch credential in memory
    only, so a dropped socket can be reopened to the same still-running window
    instead of stranding it (a launched window is not in the discovery list)."""

    def __init__(self, session, drain, process, address=None, token=None, factory=None):
        self._session, self._drain, self._process = session, drain, process
        self._address, self._factory = address, factory
        self._token = bytearray(token or "", "ascii")

    def __getattr__(self, name):
        return getattr(self._session, name)

    def reconnect(self):
        if self._address is None or not self._token or self._factory is None:
            raise Rejection("reconnect_unavailable", "This live session cannot be reopened.")
        try:
            self._session.close()
        except Exception:
            pass
        self._session = self._factory(self._address, self._token.decode("ascii"),
                                      timeout=SESSION_TIMEOUT)

    def close(self):
        try:
            self._session.close()
        finally:
            self._token[:] = b"\0" * len(self._token)
            self._token.clear()
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
        if deadline - time.monotonic() <= 0:
            raise TimeoutError()
        factory = session_factory or _sdk().LiveSession
        # The startup budget bounds only startup; every later request gets the
        # full session timeout (apply_and_verify extends it per request).
        session = factory(address, token, timeout=SESSION_TIMEOUT)
        return _LaunchedSession(session, drain, process, address, token, factory)
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
    def __init__(self, plan_state, launcher=None, discoverer=None, attacher=None):
        self.plan_state = plan_state
        self.launcher, self.sdk = launcher or _launch, _live()
        self.discoverer = discoverer or self.sdk.list_live_instances
        self.attacher = attacher or self.sdk.attach_live_instance
        self.sessions = {}
        self.lock = asyncio.Lock()

    def guard(self):
        # Hosts without a plan mode (no binding) may always write.
        if getattr(self.plan_state, "active", False) is True:
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

    def reopen(self, handle):
        """Reopen a launched window's socket in place; False if not possible."""
        reconnect = getattr(self.sessions.get(handle), "reconnect", None)
        if not callable(reconnect):
            return False
        try:
            reconnect()
            self.sessions[handle].status()
            return True
        except Exception:
            return False

    def prune_closed(self):
        for handle, session in list(self.sessions.items()):
            if getattr(session, "closed", False) is True:
                self.forget(handle)

    def expected(self, value):
        # Optional: a stamp from any earlier response guards against concurrent human edits.
        return _live()._stamp(value)

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
                elif isinstance(error, self.sdk.LiveConsentError):
                    value = _error(error.code, "The target window did not grant live access.")
                elif isinstance(error, self.sdk.LiveBridgeError):
                    details = error.details or {}
                    message = details.get("message") or _REJECTION_HINTS.get(
                        error.code, "Live bridge rejected the request.")
                    value = _error(error.code, message, details=error.details)
                elif isinstance(error, self.sdk.LiveTransportError):
                    unknown = bool(error.mutation_outcome_unknown)
                    if self.reopen(handle):
                        value = _error("live_transport_error",
                                       "Live connection dropped and was reopened on the same handle. "
                                       "Re-read status before any mutation" + (
                                           "; the last mutation may or may not have applied." if unknown else "."),
                                       unknown, details={"reconnected": True})
                    else:
                        self.forget(handle)
                        value = _error("live_transport_error", "Live connection failed. Do not retry mutations.",
                                       unknown)
                elif isinstance(error, (ValueError, TypeError)):
                    value = _error("invalid_arguments", "Invalid live tool arguments; no automatic retry.", mutation)
                else:
                    self.forget(handle)
                    value = _error("live_operation_failed", "Live operation failed; details suppressed. Do not retry mutations.", mutation)
                return _output(value)


def register_tools() -> list:
    return _register_tools(_plan_state())


def _register_tools(plan_state, *, launcher=None, discoverer=None, attacher=None) -> list:
    """Python host/test injection only; not exposed in any tool schema."""
    runtime = Runtime(plan_state, launcher, discoverer, attacher)

    @beta_async_tool(name="KetchupLiveSession")
    async def session(action: str, handle: str = "", instance_id: str = "",
                      executable: str = "", document_path: str = "") -> str:
        """List, attach to an approved open window, launch, or disconnect. Never terminates the GUI.

        Args:
            action: list, attach, launch, or disconnect. Attach/launch are forbidden in plan mode.
            handle: Live UUID, required only for disconnect (also allowed in plan mode).
            instance_id: Listed 32-hex window ID, required only for attach; confirmation occurs in that window.
            executable: Explicit absolute existing GUI executable, required for launch. No discovery or extra arguments.
            document_path: Optional absolute existing document file for the new window; never replaces an existing window.
        """
        owned = str(uuid.uuid4()) if action in ("attach", "launch") else handle
        def job():
            _action(action, ("list", "attach", "launch", "disconnect"))
            if action == "list":
                if handle or instance_id or executable or document_path:
                    raise Rejection("invalid_arguments", "List accepts no session, instance, executable, or document arguments.")
                return {"ok": True, "result": {"instances": runtime.discoverer(), "complete": True}}
            if action == "disconnect":
                if instance_id or executable or document_path:
                    raise Rejection("invalid_arguments", "Disconnect accepts only a live handle.")
                live_session = runtime.entry(handle)
                try:
                    if live_session.disconnect() is None:
                        raise Rejection("live_transport_error", "Session already closed; remote disconnection was not confirmed.")
                finally:
                    runtime.forget(handle)
                return {"ok": True, "result": {"disconnected": handle, "app_terminated": False}}
            runtime.guard()
            if action == "attach":
                if handle or executable or document_path:
                    raise Rejection("invalid_arguments", "Attach accepts only one listed instance ID.")
                runtime.prune_closed()
                if len(runtime.sessions) >= MAX_SESSIONS:
                    raise Rejection("session_limit", "Disconnect a live session first; maximum is four.")
                live_session = runtime.attacher(instance_id)
                runtime.sessions[owned] = live_session
                try:
                    runtime.guard()
                    result = live_session.status()
                    return {**result, "result": {**result["result"], "handle": owned,
                            "ownership": "nonowning_existing_GUI_window", "plan_guard_bound": True,
                            "attachment": "Approved directly in the selected existing window."}}
                except BaseException:
                    try:
                        live_session.disconnect()
                    except Exception:
                        pass
                    runtime.forget(owned)
                    raise
            if instance_id:
                raise Rejection("invalid_arguments", "Launch accepts no existing instance ID.")
            if handle:
                raise Rejection("invalid_arguments", "Launch creates a new handle and new GUI window.")
            runtime.prune_closed()
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
                      workset_handle: str = "", operation: str = "") -> str:
        """Read live status/summary/operations/query/detail. Results retain the complete bridge stamp; no geometry is fabricated.

        Args:
            action: status, summary, operations, query, detail, workset_create, or workset_status. Allowed in plan mode. operations lists every CAD program operation with its fields straight from Kečup; add operation=<name> for one operation plus all types it references. Use it instead of reading source code.
            operation: Operation name for action=operations (e.g. create_panel); empty lists all.
            handle: Live session UUID.
            expected: Ignored for reads; accepted for symmetry.
            kind: occurrences, instances, definitions, features, relations, faces, or edges. Topology rows include stable reference IDs and exact geometry.
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
            _action(action, ("status", "summary", "operations", "query", "detail", "workset_create", "workset_status"))
            if operation and action != "operations":
                raise Rejection("invalid_arguments", "operation applies only to action=operations.")
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
            if action == "operations":
                return live_session.operations(operation)
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
    async def edit(action: str, handle: str, expected: dict | None = None,
                   selection: list[int] | None = None,
                   program: dict | None = None, proposal_id: int = 0) -> str:
        """Propose/commit/undo/redo. Never retry mutations after a transport error.

        Args:
            action: propose, commit, undo, or redo; all forbidden in plan mode.
            handle: Live session UUID.
            expected: Optional stamp from an earlier response; rejects with stale_document if the document changed since.
            selection: Optional root occurrence IDs the GUI selection must equal.
            program: For propose only, typed CAD program object containing operations; Rust validates and plans it.
            proposal_id: For commit only, positive proposal ID returned on this connection.
        """
        def job():
            _action(action, ("propose", "commit", "undo", "redo"))
            runtime.guard()
            live_session = runtime.entry(handle)
            stamp = runtime.expected(expected)
            if action == "propose":
                return live_session.propose(stamp, selection, program)
            if action == "commit":
                return live_session.commit(stamp, proposal_id)
            return getattr(live_session, action)(stamp)
        return await runtime.run(handle, job, mutation=True)

    @beta_async_tool(name="KetchupLiveModel")
    async def model(action: str, handle: str, expected: dict | None = None,
                    targets: list[dict] | None = None,
                    selection: list[int] | None = None,
                    program: dict | None = None, validators: list[str] | None = None,
                    timeout_ms: int = 60_000, save: dict | None = None, strict: bool = False) -> str:
        """Read a narrow semantic edit context or apply one program as a verified single Undo step.

        One user request = one apply_and_verify; its validation IS the verification (no tests, no images).
        apply_and_verify publishes the edit and reports validator issues (validation.issues with
        the parts involved); fix them with a follow-up edit. strict=true rejects instead.
        Full catalog: KetchupLiveInspect action=operations [operation=<name>]. Verified examples:
        program = {"operations":[OP, ...]}; lengths mm; ids come from query/status/diff.targets.
        create_panel: {"operation":"create_panel","name":"Lem predny","dimensions_mm":[500,18,350],
          "holes":[],"translation_mm":[0,0,450]}  (box 0..dims in local axes; rotation? {pivot_mm,axis,angle_degrees})
          create_panel [a,b,c] = sketch a x b (feature id+1: bounds.width=a, bounds.height=b) + pad c (id+2: extent.distance=c).
        set_feature_parameter (resize): {"operation":"set_feature_parameter","feature_id":2,
          "parameter_path":"bounds.height","value_type":"length","value":15}  (paths/ids: edit_context targets=[{root_occurrence_id,steps:[]}])
        transform: {"operation":"transform","selector":{"type":"occurrences","occurrence_ids":[54]},"translation_mm":[0,0,10]}
        delete: {"operation":"delete","selector":{"type":"occurrences","occurrence_ids":[54]},
          "dependency_policy":"remove_references"}   (or "reject_if_referenced")
        create_physical_dowel_joint: {"operation":"create_physical_dowel_joint","name":"Roh","first":FACE,"second":FACE,
          "first_center_local_mm":[x,y,z],"row_unit_first_local":[0,0,1],"count":2,"spacing_mm":250,"dowel":"d8x30"}
          FACE={"instance_path":{"root_occurrence_id":N,"steps":[]},"face_origin_local_mm":[..],"inward_unit_local":[..],
          "bounds_min_local_mm":[0,0,0],"bounds_max_local_mm":dims}; both faces in one world plane; center in first
          part's local frame; dowels d6x30|d8x30|d8x40|d10x40; first_insertion_mm? for boards < 16 mm thick.
        move_physical_dowel_pair: {"operation":"move_physical_dowel_pair","joint_id":8,"pair_index":1,
          "offset_first_local_mm":[35,0,0]}  (offset from the evenly spaced row position)
        delete_physical_dowel_joint: {"operation":"delete_physical_dowel_joint","joint_id":11}

        Args:
            action: edit_context or apply_and_verify.
            handle: Live session UUID.
            expected: Optional stamp from an earlier response; rejects with stale_document if the document changed since.
            targets: For edit_context, 1 to 8 observed instance paths using stable root/local IDs.
            selection: Optional root occurrence IDs the GUI selection must equal.
            program: For apply_and_verify, one typed semantic CAD patch containing operations.
            validators: Optional extra validator IDs; collision always runs; pass ["gravity_support"] for an occasional gravity check.
            timeout_ms: Whole host-job deadline from 1 through 120000 ms (default 60000); on timeout nothing is applied.
            save: Optional tagged save request: {"mode":"current"} or {"mode":"path","path":"..."}.
            strict: Reject the edit when a validator fails instead of publishing it with issues.
        """
        def job():
            _action(action, ("edit_context", "apply_and_verify"))
            live_session = runtime.entry(handle)
            stamp = runtime.expected(expected)
            if action == "edit_context":
                if (selection is not None or program is not None
                        or validators is not None or timeout_ms != 60_000 or save is not None or strict):
                    raise Rejection("invalid_arguments", "edit_context accepts only targets.")
                return live_session.edit_context(stamp, targets)
            runtime.guard()
            if targets is not None or program is None:
                raise Rejection("invalid_arguments", "apply_and_verify requires a program.")
            return live_session.apply_and_verify(
                program, expected=stamp, selection=selection, validators=validators,
                timeout_ms=timeout_ms, save=save, strict=strict,
            )
        return await runtime.run(handle, job, mutation=action == "apply_and_verify")

    @beta_async_tool(name="KetchupLiveFile")
    async def file(action: str, handle: str, expected: dict | None = None, path: str = "") -> str:
        """Save or open the attached GUI document through its native file workflow.

        Args:
            action: save, save_as, or open. Save requires an existing document path; save_as requires overwrite confirmation; open requires discard confirmation when dirty.
            handle: Live session UUID.
            expected: Optional stamp from an earlier response guarding against concurrent edits.
            path: Explicit absolute destination for save_as or existing source for open; empty for save.
        """
        def job():
            _action(action, ("save", "save_as", "open"))
            runtime.guard()
            live_session = runtime.entry(handle)
            stamp = runtime.expected(expected)
            if action == "save":
                if path:
                    raise Rejection("invalid_path", "Path is only valid for save_as or open.")
                return live_session.save(stamp)
            if action == "save_as":
                return live_session.save_as(stamp, _destination(path))
            return live_session.open(stamp, _path(path))
        return await runtime.run(handle, job, mutation=True)

    @beta_async_tool(name="KetchupLiveBatch")
    async def batch(action: str, handle: str, expected: dict | None = None,
                    workset_handle: str = "", job_handle: str = "",
                    operation: dict | None = None) -> str:
        """Run one bounded live occurrence-workset batch job step at a time.

        Args:
            action: start, status, step, or cancel.
            handle: Live session UUID.
            expected: Optional stamp from an earlier response guarding against concurrent edits.
            workset_handle: Complete occurrence workset handle required only for start.
            job_handle: Opaque batch job handle required for status/step/cancel.
            operation: For start only; currently {"type":"set_color","color":[r,g,b] or null}.
        """
        def job():
            _action(action, ("start", "status", "step", "cancel"))
            if action in ("start", "step"):
                runtime.guard()
            live_session = runtime.entry(handle)
            stamp = runtime.expected(expected)
            if action == "start":
                if (not workset_handle or len(workset_handle) > 4096 or job_handle
                        or not isinstance(operation, dict)
                        or len(json.dumps(operation, allow_nan=False).encode("utf-8")) > 4096):
                    raise Rejection("invalid_arguments", "start requires one bounded workset handle and operation object.")
                runtime.guard()
                return live_session.start_batch_job(stamp, workset_handle, operation)
            if (not job_handle or len(job_handle) > 128 or workset_handle or operation is not None):
                raise Rejection("invalid_arguments", "status/step/cancel require only one bounded job handle.")
            if action == "status":
                return live_session.batch_job_status(stamp, job_handle)
            if action == "cancel":
                return live_session.cancel_batch_job(stamp, job_handle)
            runtime.guard()
            return live_session.step_batch_job(stamp, job_handle)
        return await runtime.run(handle, job, mutation=action == "step")

    @beta_async_tool(name="KetchupLiveView")
    async def view(action: str, handle: str, expected: dict | None = None,
                   occurrence_ids: list[int] | None = None, view: str = "", image_path: str = "",
                   capture_mode: str = "offscreen", max_side_px: int = 512,
                   framing: str = "viewport", detail_occurrence_id: int = 0,
                   detail_kind: str = "", detail_entity_id: int = 0) -> str:
        """Guarded live view commands and CAD PNG artifacts; never desktop screenshots.

        Saving an artifact does not establish visual delivery or geometry correctness.

        Args:
            action: selection, view, or image. All are forbidden in plan mode (image writes a file).
            handle: Live session UUID.
            expected: Optional stamp from an earlier response guarding against concurrent edits.
            occurrence_ids: Explicit root occurrence IDs for selection, including [] to clear.
            view: For view action, iso, top, front, or zoom_fit.
            image_path: Required only for image: explicit absolute NEW .png under workspace artifacts/live-view; never overwritten.
            capture_mode: For image, offscreen (default AI render) or visible_viewport (proof of a visible focused canvas).
            max_side_px: For image, maximum output side from 512 through 1600 pixels (default 512).
            framing: For image, viewport, selection, or detail_selection for an explicit host-issued topology detail.
            detail_occurrence_id: Root occurrence containing the detail; required only for detail_selection.
            detail_kind: edges or faces; required only for detail_selection.
            detail_entity_id: Positive host-issued ID from the matching live topology query/detail; required only for detail_selection.
        """
        def job():
            _action(action, ("selection", "view", "image"))
            runtime.guard()
            if action != "image" and (
                    image_path or capture_mode != "offscreen" or max_side_px != 512
                    or framing != "viewport" or detail_occurrence_id != 0
                    or detail_kind or detail_entity_id != 0):
                raise Rejection(
                    "invalid_arguments",
                    "Image capture and detail-framing arguments apply only to image.",
                )
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
                response = live_session.image(
                    stamp, capture_mode=capture_mode, max_side_px=max_side_px, framing=framing,
                    detail_occurrence_id=detail_occurrence_id, detail_kind=detail_kind,
                    detail_entity_id=detail_entity_id,
                )
                runtime.guard()  # Plan may have changed while the bounded read ran.
                try:
                    return runtime.sdk.save_image(
                        response,
                        stamp,
                        str(destination),
                        capture_mode=capture_mode,
                        max_side_px=max_side_px,
                        framing=framing,
                        detail_occurrence_id=detail_occurrence_id,
                        detail_kind=detail_kind,
                        detail_entity_id=detail_entity_id,
                    )
                except FileExistsError:
                    raise Rejection("file_exists", "Image destination exists; choose a NEW .png path.") from None
                except runtime.sdk.LiveProtocolError:
                    # Local validation of an already-received image; the connection is fine.
                    raise Rejection("invalid_image", "The rendered image failed local validation; nothing was saved.") from None
            runtime.guard()
            if action == "selection":
                return live_session.selection(stamp, occurrence_ids)
            return live_session.view(stamp, view)
        return await runtime.run(handle, job, mutation=action in ("selection", "view"))

    return [session, inspect, edit, model, file, batch, view]
