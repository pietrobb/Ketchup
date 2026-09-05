"""Supervisor-first, owned offline CAD sessions; never attaches to the GUI."""
from __future__ import annotations

import asyncio
import importlib.util
import inspect
import json
import os
from pathlib import Path
import sys
import uuid

from anthropic.lib.tools import beta_async_tool

MAX_OUTPUT = 32 * 1024
MAX_SESSIONS = 4


def _sdk():
    # Private package: no sys.path edits or shadowing a global ketchup package.
    name = "_supervisor_ketchup_model_sdk"
    if name not in sys.modules:
        root = Path(__file__).resolve().parents[1] / "sdk" / "python" / "ketchup"
        spec = importlib.util.spec_from_file_location(
            name, root / "__init__.py", submodule_search_locations=[str(root)])
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        try:
            spec.loader.exec_module(module)
        except BaseException:
            sys.modules.pop(name, None)
            raise
    return sys.modules[name]


def _plan_state():
    # src/claude_engine.py creates _plan_state before synchronous load_skills().
    # Capture the shared instance, NOT PlanModeState.active on the class.
    frame = inspect.currentframe()
    try:
        while frame:
            owner = frame.f_locals.get("self")
            if (frame.f_globals.get("__name__", "").split(".")[-1] == "claude_engine"
                    and type(owner).__name__ == "ClaudeEngine"):
                return getattr(owner, "_plan_state", None)
            frame = frame.f_back
    finally:
        del frame
    return None


class Rejection(ValueError):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code


def _json(value):
    return json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(",", ":"))


def _output(value):
    text = _json(value)
    if len(text.encode("utf-8")) <= MAX_OUTPUT:
        return text
    return _json({"ok": False, "error": {"code": "output_too_large",
                  "message": "Complete result exceeds 32 KiB; not returned. Narrow the query. "
                             "Verification is incomplete; do not infer a pass. Do not retry mutations."},
                  "complete": False, "operation_completed": value.get("ok", False)})


def _absolute(path):
    value = Path(path).expanduser()
    if not path or not value.is_absolute() or len(path) > 4096:
        raise Rejection("invalid_path", "Supply an explicit absolute path (max 4096 characters)")
    return str(value.resolve())


def _action(value, choices):
    if value not in choices:
        raise Rejection("invalid_action", "Choose one of: " + ", ".join(choices))


def _identity(state):
    return {key: state[key] for key in ("document_id", "revision", "canonical_digest")}


def _precondition(entry, fresh, revision, digest):
    if type(revision) is not int or revision < 0 or not digest:
        raise Rejection("precondition_required", "Supply expected_revision and expected_digest from an inspect read")
    expected = {"document_id": entry["document_id"], "revision": revision, "canonical_digest": digest}
    if entry.get("observed") != expected or _identity(fresh) != expected:
        raise Rejection("stale_precondition", "Plan is stale or unobserved; inspect before another mutation")


def _capability(caps, section, name):
    definitions = caps["cad_program_schema"]["$defs"]
    variants = definitions["AssistantCadEditOperation"]["oneOf"]
    operations = {v["properties"]["operation"]["const"]: v for v in variants}
    if section == "methods":
        return {key: value for key, value in caps.items() if key != "cad_program_schema"}
    if section == "operations":
        return {"operations": sorted(operations), "definitions": sorted(definitions),
                "schema_usage": "Request section=operation or definition, name=one listed name. "
                                "Resolve #/$defs/X using section=definition,name=X."}
    catalog = operations if section == "operation" else definitions
    if name not in catalog:
        raise Rejection("unknown_capability", "Unknown operation/definition; discover operations first")
    return {"name": name, "schema": catalog[name], "references": "Resolve #/$defs/X via section=definition,name=X"}


class Runtime:
    def __init__(self, plan_state):
        self.plan_state = plan_state
        self.sessions = {}
        self.lock = asyncio.Lock()

    def guard(self):
        if self.plan_state is None:
            raise Rejection("plan_guard_unavailable", "No Supervisor plan state binding; mutations disabled")
        if self.plan_state.active:
            raise Rejection("plan_mode", "CAD mutations are forbidden in Supervisor plan mode")

    def entry(self, handle):
        try:
            if str(uuid.UUID(handle)) != handle:
                raise ValueError()
            return self.sessions[handle]
        except (ValueError, KeyError, AttributeError):
            raise Rejection("invalid_handle", "Unknown or closed owned session UUID") from None

    def forget(self, handle):
        entry = self.sessions.pop(handle, None)
        if entry:
            entry["session"].close()

    def read(self, entry):
        result = entry["document"].summary()
        state = result["state"]
        if state["document_id"] != entry["document_id"]:
            raise Rejection("document_changed", "Owned document identity changed")
        return result

    def summary(self, entry, result):
        state = result["state"]
        return {**result["summary"], "identity": _identity(state), "path": entry["path"],
                "unsaved": entry["saved_digest"] != state["canonical_digest"],
                "undo_steps": state["undo_steps"], "redo_steps": state["redo_steps"],
                "backend_compact": True}

    def mutation(self, handle, revision, digest):
        self.guard()
        entry = self.entry(handle)
        _precondition(entry, self.read(entry)["state"], revision, digest)
        self.guard()
        return entry

    async def run(self, handle, job):
        async with self.lock:
            task = asyncio.create_task(asyncio.to_thread(job))
            try:
                return _output({"ok": True, "result": await asyncio.shield(task)})
            except asyncio.CancelledError:
                # Retain the lock through repeated cancellation and owned-process cleanup.
                async def drain(pending):
                    while True:
                        try:
                            await asyncio.shield(pending)
                            break
                        except asyncio.CancelledError:
                            if pending.cancelled(): break
                        except Exception: break
                await drain(task)
                if handle: await drain(asyncio.create_task(asyncio.to_thread(self.forget, handle)))
                raise
            except Exception as error:
                if isinstance(error, _sdk().TransportError) and handle:
                    await asyncio.to_thread(self.forget, handle)
                return _output({"ok": False, "error": {
                    "code": getattr(error, "code", type(error).__name__),
                    "message": str(error), "details": getattr(error, "details", None)},
                    "retry_mutation": False})


def register_tools() -> list:
    runtime = Runtime(_plan_state())

    @beta_async_tool(name="KetchupDiscover")
    async def discover(section: str = "overview", handle: str = "", name: str = "") -> str:
        """Discover owned offline CAD tools, then one capability section, never the full schema.

        Args:
            section: overview, methods, operations, operation, definition, or validators.
            handle: Owned session UUID; required except for overview.
            name: Exact operation or schema definition name when requesting its schema.
        """
        def job():
            _action(section, ("overview", "methods", "operations", "operation", "definition", "validators"))
            if section == "overview":
                return {"mode": "owned_headless_not_live_GUI", "max_sessions": MAX_SESSIONS,
                        "max_output_bytes": MAX_OUTPUT, "plan_guard_bound": runtime.plan_state is not None,
                        "backend_compact": True,
                        "workflow": "session new/open -> inspect summary -> search -> detail -> edit -> inspect -> verify -> save",
                        "preconditions": "edit/save/close require caller-observed expected_revision and expected_digest; inspect after every edit",
                        "limitations": "Requires compact-capable SDK/headless. No scripts, raw canonical commands, "
                                       "GUI bridge, spatial queries or automatic retries. Oversized evidence is rejected incomplete."}
            entry = runtime.entry(handle)
            if section == "validators":
                return entry["document"].validators.list()
            return _capability(entry["session"].capabilities(), section, name)
        return await runtime.run(handle, job)

    @beta_async_tool(name="KetchupSession")
    async def session(action: str, handle: str = "", path: str = "", executable: str = "",
                      worker: str = "", discard: bool = False, expected_revision: int = -1,
                      expected_digest: str = "") -> str:
        """Open/new a separate owned process and document, or safely close its UUID. Never controls the GUI.

        Args:
            action: new, open, or close. New/open never replaces another handle.
            handle: Only for close; UUID returned by new/open.
            path: Absolute document path, only for open.
            executable: Explicit absolute headless executable, or existing KETCHUP_HEADLESS env.
            worker: Optional explicit absolute exact-worker path; otherwise SDK worker resolution.
            discard: Required to close any unsaved document, including an unsaved new one.
            expected_revision: Observed inspect revision, required for close.
            expected_digest: Observed inspect canonical_digest, required for close.
        """
        owned = str(uuid.uuid4()) if action in ("new", "open") else handle
        def job():
            _action(action, ("new", "open", "close"))
            if action == "close":
                if path or executable or worker:
                    raise Rejection("invalid_arguments", "close accepts no paths or executables")
                entry = runtime.mutation(handle, expected_revision, expected_digest)
                if entry["saved_digest"] != entry["observed"]["canonical_digest"] and not discard:
                    raise Rejection("unsaved_changes", "Use discard=true to close an unsaved document")
                runtime.forget(handle)
                return {"closed": handle}
            runtime.guard()
            if handle or discard or (action == "new" and path):
                raise Rejection("invalid_arguments", "new/open require a separate document, no handle or discard")
            if len(runtime.sessions) >= MAX_SESSIONS:
                raise Rejection("session_limit", "Close an owned session first; maximum is four")
            document_path = _absolute(path) if action == "open" else None
            binary = _absolute(executable) if executable else os.environ.get("KETCHUP_HEADLESS")
            if not binary:
                raise Rejection("executable_required", "Supply executable or KETCHUP_HEADLESS; no automatic PATH binary selection")
            exact = _absolute(worker) if worker else None
            sdk_session = _sdk().Session(executable=binary, worker=exact, compact=True)
            runtime.sessions[owned] = {"session": sdk_session}
            try:
                document = sdk_session.open_document(document_path) if document_path else sdk_session.new_document()
                result = document.summary()
                state = result["state"]
                entry = runtime.sessions[owned]
                entry.update(document=document, document_id=state["document_id"], path=document_path,
                             saved_digest=state["canonical_digest"] if document_path else None,
                             observed=_identity(state))
                return {"handle": owned, "ownership": "owned_headless_not_live_GUI", **runtime.summary(entry, result)}
            except BaseException:
                runtime.forget(owned)
                raise
        return await runtime.run(owned, job)

    @beta_async_tool(name="KetchupInspect")
    async def inspect_model(handle: str, action: str = "summary", kind: str = "occurrences",
                            search: str = "", entity_id: int = 0, cursor: str = "", limit: int = 20,
                            definition_id: int | None = None) -> str:
        """Read macro summary first, search paged metadata, then detail by ID; no geometry evaluation.

        Args:
            handle: Owned session UUID.
            action: summary, search, or detail.
            kind: occurrences, definitions, or features.
            search: Case-sensitive name substring, at most 128 UTF-8 bytes.
            entity_id: Positive ID for detail.
            cursor: Opaque next_cursor from previous search; keep query unchanged. Stale tokens are rejected.
            limit: Search page size from 1 to 100.
            definition_id: Optional positive definition filter for occurrence/feature search.
        """
        def job():
            _action(action, ("summary", "search", "detail"))
            _action(kind, ("occurrences", "definitions", "features"))
            if not 1 <= limit <= 100 or len(cursor) > 4096 or len(search.encode("utf-8")) > 128:
                raise Rejection("invalid_query", "Invalid page bounds or search too long")
            entry = runtime.entry(handle)
            if action == "summary":
                result = runtime.summary(entry, runtime.read(entry))
            elif action == "detail":
                if entity_id <= 0:
                    raise Rejection("invalid_query", "Detail requires a positive entity ID")
                result = entry["document"].detail(kind, entity_id)
            else:
                result = entry["document"].query(kind=kind, search=search, limit=limit,
                                               cursor=cursor or None, definition_id=definition_id)
            if result["identity"]["document_id"] != entry["document_id"]:
                raise Rejection("document_changed", "Owned document identity changed")
            if len(_json({"ok": True, "result": result}).encode("utf-8")) <= MAX_OUTPUT:
                entry["observed"] = _identity(result["identity"])
            return result
        return await runtime.run(handle, job)

    @beta_async_tool(name="KetchupEdit")
    async def edit(handle: str, action: str, expected_revision: int, expected_digest: str,
                   program: dict | None = None, selection: list[int] | None = None) -> str:
        """Apply one atomic typed CAD program, undo, or redo. Inspect first; never silently refresh a stale plan.

        Args:
            handle: Owned session UUID.
            action: apply, undo, or redo.
            expected_revision: Revision from previous inspect/session read.
            expected_digest: canonical_digest from that same read.
            program: Public CadEditProgram object with operations; only for apply. No code or file execution.
            selection: Explicit occurrence IDs for current_selection, only for apply; never GUI selection.
        """
        def job():
            _action(action, ("apply", "undo", "redo"))
            if action == "apply":
                if not isinstance(program, dict) or len(_json(program).encode("utf-8")) > 128 * 1024:
                    raise Rejection("invalid_program", "Supply a finite typed program of at most 128 KiB")
            elif program is not None or selection is not None:
                raise Rejection("invalid_arguments", "undo/redo accept no program or selection")
            entry = runtime.mutation(handle, expected_revision, expected_digest)
            entry["observed"] = None
            doc = entry["document"]
            result = doc.apply(program, selection=selection or []) if action == "apply" else getattr(doc, action)()
            return {"summary": runtime.summary(entry, result), "created": result.get("created", {}),
                    "next": "inspect before another mutation"}
        return await runtime.run(handle, job)

    @beta_async_tool(name="KetchupSave")
    async def save(handle: str, path: str, expected_revision: int, expected_digest: str,
                   overwrite: bool = False) -> str:
        """Save only to an explicit absolute path. Existing files require explicit overwrite=true.

        Args:
            handle: Owned session UUID.
            path: Absolute destination; never inferred from document origin.
            expected_revision: Revision from previous inspect/session read.
            expected_digest: canonical_digest from the same read.
            overwrite: Explicit permission to replace an existing destination; defaults false.
        """
        def job():
            destination = _absolute(path)
            entry = runtime.mutation(handle, expected_revision, expected_digest)
            if Path(destination).exists() and not overwrite:
                raise Rejection("file_exists", "Destination exists; overwrite=true is required")
            result = entry["document"].save(destination, overwrite=overwrite)
            entry.update(path=destination, saved_digest=result["state"]["canonical_digest"], observed=None)
            return runtime.summary(entry, result)
        return await runtime.run(handle, job)

    @beta_async_tool(name="KetchupVerify")
    async def verify(handle: str, action: str = "evaluate", validator_ids: list[str] | None = None) -> str:
        """Return actual exact/validator evidence unchanged. Tool ok is NOT a geometry pass; check report completeness.

        Args:
            handle: Owned session UUID.
            action: evaluate, validators, or list.
            validator_ids: Explicit validator IDs for validators; discover/list first.
        """
        def job():
            _action(action, ("evaluate", "validators", "list"))
            doc = runtime.entry(handle)["document"]
            if action == "validators":
                if not validator_ids or len(validator_ids) > 100:
                    raise Rejection("invalid_validators", "Supply 1..100 validator IDs")
                return doc.validators.run(validator_ids)
            if validator_ids is not None:
                raise Rejection("invalid_arguments", "validator_ids applies only to validators")
            return doc.evaluate(timeout_ms=30000) if action == "evaluate" else doc.validators.list()
        return await runtime.run(handle, job)

    return [discover, session, inspect_model, edit, save, verify]
