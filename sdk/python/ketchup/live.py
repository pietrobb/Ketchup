"""Non-owning S4a host bridge client; not production application discovery/attach.

A trusted embedding host must explicitly enable the otherwise disabled bridge
and supply its numeric loopback address and OS-random credential out of band.
No environment discovery, subprocesses, document ownership, or geometry evaluation.
Methods return the complete validated response envelope (including its stamp).
Pass that stamp explicitly on guarded calls; nothing refreshes it or retries.
Image artifacts are explicit, create-only files. Closing only closes our socket.
Secret storage is overwritten on close, but Python cannot guarantee erasure of
immutable temporary strings/bytes or of the caller's credential copy.
"""
from __future__ import annotations

from dataclasses import dataclass, asdict
import base64
import binascii
import hashlib
import os
from pathlib import Path
import stat
import zlib
import json
import math
import socket
import struct
import threading
import time
from typing import Any, Literal

from .client import ProtocolError, SessionClosedError, TransportError, TransportTimeout

MAX_FRAME_BYTES = 32768
MAX_TIMEOUT = 30.0
_U64_MAX = (1 << 64) - 1
_KINDS = ("occurrences", "instances", "definitions", "features", "relations")
_VIEWS = ("iso", "top", "front", "zoom_fit")
_CAPTURE_MODES = ("offscreen", "visible_viewport")
_MUTATIONS = frozenset({"batch_job_step", "propose", "commit", "undo", "redo", "selection", "view"})
# Never surface arbitrary remote text, even if it looks like an error code.
_ERROR_CODES = frozenset({
    "invalid_request", "unauthorized", "unsupported_version", "queue_unavailable",
    "response_limit", "stale_document", "unsupported_selection_scope",
    "selection_limit", "invalid_selection", "read_only_document", "selection_changed",
    "invalid_program", "planning_rejected", "proposal_ids_exhausted",
    "receipt_guard_mismatch", "proposal_not_found", "commit_rejected",
    "undo_unavailable", "redo_unavailable", "entity_not_found", "view_unavailable",
    "unsupported_image", "invalid_params", "invalid_cursor", "stale_cursor",
    "cross_query_cursor", "output_too_large", "busy", "image_unavailable", "image_timeout",
    "hidden_viewport", "stale_image", "occluded_viewport", "invalid_image_callback",
    "invalid_image_dimensions", "incomplete_image", "unsupported_image_texture",
    "unsupported_image_renderer", "stale_workset", "workset_not_found",
    "unsupported_workset_scope", "incomplete_workset", "missing_workset_identity",
    "batch_job_limit", "batch_job_ids_exhausted", "batch_job_not_found",
    "batch_cancelled", "stale_batch_task", "batch_transaction_failed",
})
_FATAL_CODES = frozenset({"invalid_request", "unauthorized", "unsupported_version", "queue_unavailable"})
Kind = Literal["occurrences", "instances", "definitions", "features", "relations"]
View = Literal["iso", "top", "front", "zoom_fit"]
CaptureMode = Literal["offscreen", "visible_viewport"]


class LiveBridgeError(RuntimeError):
    """Definite server rejection; only a locally allowlisted code is exposed."""

    def __init__(self, code: str):
        self.code = code if code in _ERROR_CODES else "remote_error"
        super().__init__("live bridge rejected request: " + self.code)


class LiveTransportError(TransportError):
    """Socket failure; a sent mutation may have executed. Never retry blindly."""

    def __init__(self, message: str, *, mutation_outcome_unknown: bool = False):
        self.mutation_outcome_unknown = mutation_outcome_unknown
        if mutation_outcome_unknown:
            message += "; mutation outcome unknown; re-observe before any further mutation"
        super().__init__(message)


class LiveProtocolError(LiveTransportError, ProtocolError):
    pass


class LiveTimeout(LiveTransportError, TransportTimeout):
    pass


def _uint(value: Any, minimum: int = 0, maximum: int = _U64_MAX) -> int:
    if type(value) is not int or not minimum <= value <= maximum:
        raise ValueError("invalid unsigned integer")
    return value


def _text(value: Any, maximum: int) -> str:
    if type(value) is not str:
        raise ValueError("invalid text")
    try:
        valid = len(value.encode("utf-8")) <= maximum
    except UnicodeError:
        valid = False
    if not valid:
        raise ValueError("invalid or oversized UTF-8 text")
    return value


@dataclass(frozen=True)
class Stamp:
    """Caller-owned guard, including epoch to distinguish undo/redo ABA states."""

    document_id: int
    revision: int
    canonical_digest: str
    mutation_epoch: int

    def __post_init__(self):
        _uint(self.document_id)
        _uint(self.revision)
        _uint(self.mutation_epoch)
        _text(self.canonical_digest, MAX_FRAME_BYTES)


def _stamp(value: Any) -> dict:
    if isinstance(value, Stamp):
        return asdict(value)
    if type(value) is not dict or set(value) != {
        "document_id", "revision", "canonical_digest", "mutation_epoch"
    }:
        raise ValueError("expected a complete live bridge stamp")
    return asdict(Stamp(**value))


def _ids(values: Any) -> list[int]:
    if type(values) not in (list, tuple) or len(values) > 100:
        raise ValueError("selection must contain at most 100 occurrence IDs")
    result = [_uint(value, 1) for value in values]
    if len(set(result)) != len(result):
        raise ValueError("selection IDs must be unique")
    return result


def _world_bounds(value: Any) -> list[list[float | int]] | None:
    if value is None:
        return None
    if (type(value) not in (list, tuple) or len(value) != 2
            or any(type(point) not in (list, tuple) or len(point) != 3 for point in value)):
        raise ValueError("invalid world bounds")
    result = []
    for point in value:
        converted = []
        for axis in point:
            if type(axis) not in (int, float):
                raise ValueError("invalid world bounds")
            try:
                finite = math.isfinite(axis)
            except OverflowError:
                finite = False
            if not finite:
                raise ValueError("invalid world bounds")
            converted.append(axis)
        result.append(converted)
    if any(result[0][axis] > result[1][axis] for axis in range(3)):
        raise ValueError("invalid world bounds")
    return result


def _model_query(kind: Kind, limit: int, search: str, definition_id: int | None,
                 tag_id: int | None, classification_dimension_id: int | None,
                 classification_category_id: int | None, cursor: str | None,
                 world_bounds_mm: list[list[float]] | None, *, allow_cursor: bool) -> dict:
    if type(kind) is not str or kind not in _KINDS:
        raise ValueError("invalid entity kind")
    _uint(limit, 1, 100)
    _text(search, 128)
    if definition_id is not None:
        _uint(definition_id, 1)
        if kind == "definitions":
            raise ValueError("definitions cannot be filtered by definition ID")
    for property_id in (tag_id, classification_dimension_id, classification_category_id):
        if property_id is not None:
            _uint(property_id, 1)
    if classification_category_id is not None and classification_dimension_id is None:
        raise ValueError("classification category requires a dimension")
    if (tag_id is not None or classification_dimension_id is not None) and kind not in ("occurrences", "instances"):
        raise ValueError("property filters apply only to occurrences and instances")
    if cursor is not None:
        _text(cursor, 4096)
        if not allow_cursor:
            raise ValueError("workset source query cannot contain a cursor")
    bounds = _world_bounds(world_bounds_mm)
    if bounds is not None and kind != "instances":
        raise ValueError("world bounds apply only to instances")
    query = {"kind": kind, "limit": limit, "search": search}
    for key, value in {
        "definition_id": definition_id,
        "tag_id": tag_id,
        "classification_dimension_id": classification_dimension_id,
        "classification_category_id": classification_category_id,
        "cursor": cursor,
        "world_bounds_mm": bounds,
    }.items():
        if value is not None:
            query[key] = value
    return query


def _json_copy(value: Any, secret: str = "", depth: int = 0, budget=None) -> Any:
    """Bounded JSON-only snapshot; also rejects secret-bearing remote strings."""
    if budget is None:
        budget = [MAX_FRAME_BYTES]
    budget[0] -= 1
    if depth > 64 or budget[0] < 0:
        raise ValueError("JSON complexity limit")
    if value is None or type(value) is bool:
        return value
    if type(value) is str:
        _text(value, MAX_FRAME_BYTES)
        if secret and secret in value:
            raise ValueError("invalid response text")
        return value
    if type(value) is int:
        if not -(1 << 63) <= value <= _U64_MAX:
            raise ValueError("JSON integer out of bounds")
        return value
    if type(value) is float and math.isfinite(value):
        return value
    if type(value) is list:
        return [_json_copy(v, secret, depth + 1, budget) for v in value]
    if type(value) is dict:
        result = {}
        for key, item in value.items():
            if type(key) is not str:
                raise ValueError("JSON object keys must be strings")
            key = _json_copy(key, secret, depth + 1, budget)
            result[key] = _json_copy(item, secret, depth + 1, budget)
        return result
    raise ValueError("expected finite JSON values")


def _object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON field")
        result[key] = value
    return result


def _no_constant(_value):
    raise ValueError("nonfinite JSON number")


def _remaining(deadline: float) -> float:
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError("live bridge deadline expired")
    return remaining


# A single 32 KiB JSON frame also bounds the encoded payload. No image chunks,
# remote paths, Pillow dependency, or desktop screenshot fallback.
MAX_PNG_BYTES = MAX_FRAME_BYTES // 4 * 3
MAX_IMAGE_DIMENSION = 2048
MAX_IMAGE_PIXELS = 1024 * 1024


def _png_dimensions(data: bytes) -> tuple[int, int]:
    """Validate bounded noninterlaced RGB/RGBA8 PNG structure and scanlines."""
    if not 57 <= len(data) <= MAX_PNG_BYTES or data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("invalid PNG")
    offset, dimensions, channels = 8, None, None
    compressed = bytearray()
    ended = idat_ended = False
    while offset < len(data):
        if offset + 12 > len(data):
            raise ValueError("truncated PNG chunk")
        size = struct.unpack_from("!I", data, offset)[0]
        end = offset + 12 + size
        if end > len(data):
            raise ValueError("truncated PNG chunk")
        kind = data[offset + 4:offset + 8]
        payload = data[offset + 8:end - 4]
        if (len(kind) != 4 or not all(65 <= b <= 90 or 97 <= b <= 122 for b in kind)
                or kind[2] & 32 or zlib.crc32(kind + payload) != struct.unpack_from("!I", data, end - 4)[0]):
            raise ValueError("invalid PNG chunk")
        if dimensions is None and kind != b"IHDR":
            raise ValueError("missing PNG header")
        if kind == b"IHDR":
            if dimensions is not None or size != 13:
                raise ValueError("invalid PNG header")
            width, height, depth, color, compression, filtering, interlace = struct.unpack("!IIBBBBB", payload)
            _uint(width, 1, MAX_IMAGE_DIMENSION)
            _uint(height, 1, MAX_IMAGE_DIMENSION)
            if (width * height > MAX_IMAGE_PIXELS or depth != 8 or color not in (2, 6)
                    or (compression, filtering, interlace) != (0, 0, 0)):
                raise ValueError("unsupported or oversized PNG")
            dimensions, channels = (width, height), 3 if color == 2 else 4
        elif kind == b"IDAT":
            if idat_ended:
                raise ValueError("noncontiguous PNG data")
            compressed.extend(payload)
        elif kind == b"IEND":
            if size or not compressed or end != len(data):
                raise ValueError("invalid PNG end")
            ended = True
        else:
            if not kind[0] & 32:  # Unknown critical chunks are not safe to ignore.
                raise ValueError("unsupported PNG chunk")
            if compressed:
                idat_ended = True
        offset = end
    if not ended or dimensions is None:
        raise ValueError("incomplete PNG")
    stride = dimensions[0] * channels + 1
    expected = stride * dimensions[1]
    decoder = zlib.decompressobj()
    pixels = decoder.decompress(bytes(compressed), expected + 1)
    if (len(pixels) != expected or not decoder.eof or decoder.unused_data
            or decoder.unconsumed_tail or any(pixels[n] > 4 for n in range(0, expected, stride))):
        raise ValueError("invalid PNG scanlines")
    return dimensions


def _image_bytes(response: dict, expected: Stamp | dict, encoded_png: str,
                 capture_mode: CaptureMode = "offscreen") -> bytes:
    """Validate capture binding and bytes independently of the payload wire key.

    Wire adapter follows live_bridge/image.rs: result.data is base64 PNG.
    Capture/render metadata is retained, never promoted to visual evidence.
    """
    if type(capture_mode) is not str or capture_mode not in _CAPTURE_MODES:
        raise ValueError("invalid capture mode")
    response = _json_copy(response)
    if len(json.dumps(response, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode("utf-8")) > MAX_FRAME_BYTES:
        raise ValueError("image response exceeds frame budget")
    if (type(response) is not dict or response.get("ok") is not True
            or response.get("error") is not None or _stamp(response.get("stamp")) != _stamp(expected)):
        raise ValueError("image stamp mismatch")
    result = response.get("result")
    if type(result) is not dict or "artifact" in result:
        raise ValueError("invalid image metadata")
    if (result.get("mime_type") != "image/png" or result.get("encoding") != "base64"
            or result.get("scope") != "cad_viewport" or "stamp" not in result
            or result.get("capture_mode") != capture_mode):
        raise ValueError("invalid CAD image metadata")
    _uint(result.get("capture_pass"))
    render = result.get("render")
    visible = capture_mode == "visible_viewport"
    if (type(render) is not dict or render.get("render_correlated") is not True
            or type(render.get("callback_correlated")) is not bool
            or render.get("viewport_visibility_required") is not visible
            or render.get("viewport_unoccluded") is not visible):
        raise ValueError("uncorrelated CAD image")
    for key in ("stamp", "capture_stamp"):
        if key in result and _stamp(result[key]) != _stamp(expected):
            raise ValueError("capture stamp mismatch")
    width = _uint(result.get("width"), 1, MAX_IMAGE_DIMENSION)
    height = _uint(result.get("height"), 1, MAX_IMAGE_DIMENSION)
    byte_count = _uint(result["byte_count"], 57, MAX_PNG_BYTES) if "byte_count" in result else None
    if type(encoded_png) is not str or not encoded_png or len(encoded_png) > (MAX_PNG_BYTES + 2) // 3 * 4:
        raise ValueError("invalid PNG encoding")
    data = base64.b64decode(encoded_png, validate=True)
    if base64.b64encode(data).decode("ascii") != encoded_png:
        raise ValueError("noncanonical PNG encoding")
    if (byte_count is not None and len(data) != byte_count) or _png_dimensions(data) != (width, height):
        raise ValueError("PNG metadata mismatch")
    return data


def _image_path(image_path: str) -> Path:
    """Explicit NEW local PNG only; refuse existing leaves and linked ancestors."""
    _text(image_path, 4096)
    path = Path(image_path)
    if (not image_path or "\0" in image_path or not path.is_absolute()
            or path.suffix.lower() != ".png" or ":" in path.name or ".." in path.parts):
        raise ValueError("supply an explicit absolute new .png path")
    for component in (path, *path.parents):
        try:
            info = component.lstat()
        except FileNotFoundError:
            continue
        if (stat.S_ISLNK(info.st_mode)
                or getattr(info, "st_file_attributes", 0) & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0)):
            raise ValueError("image paths must not contain links")
    if os.path.lexists(path):
        raise FileExistsError("image destination already exists")
    return path


def save_image(response: dict, expected: Stamp | dict, image_path: str,
               capture_mode: CaptureMode = "offscreen") -> dict:
    """Save validated CAD pixels, exclusively. Receipt is NOT visual/geometry proof.

    Missing directories on the caller's explicit path may be created, but no
    filename is inferred. Only the skill imposes workspace artifact confinement.
    """
    path = _image_path(image_path)
    try:
        snapshot = _json_copy(response)
        result = snapshot["result"]
        data = _image_bytes(snapshot, expected, result["data"], capture_mode)
        del result["data"]
        result["artifact"] = {"path": str(path), "byte_count": len(data),
                              "sha256": hashlib.sha256(data).hexdigest(),
                              "artifact_saved": True, "visual_delivery": "unverified",
                              "geometry_evaluated": False}
        if len(json.dumps(snapshot, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode("utf-8")) > MAX_FRAME_BYTES:
            raise ValueError("image receipt exceeds output budget")
    except (ValueError, TypeError, KeyError, binascii.Error, zlib.error):
        raise LiveProtocolError("invalid live image response") from None
    path.parent.mkdir(parents=True, exist_ok=True)
    _image_path(image_path)  # Recheck after mkdir; exclusive create is authoritative.
    with path.open("xb") as stream:
        if stream.write(data) != len(data):
            raise OSError("incomplete image artifact write")
    return snapshot


class LiveSession:
    """One serialized socket to a trusted host, never an owned app/process.

    ``address`` is ("127.0.0.1", port) or "127.0.0.1:port"; no DNS or discovery.
    ``token`` is the host's 64 lowercase hex characters, required on every call.
    ``timeout`` is a finite number in (0, 30] seconds, a total deadline covering
    lock wait, writes, header and body (not a resettable per-recv timeout).
    Construction connects only; it sends no implicit status or auth request.
    ``close``/context exit close locally without sending; ``disconnect`` sends
    the authenticated revocation request then closes even if it fails.
    """

    __slots__ = ("_socket", "_token", "_timeout", "_lock", "_id", "_closed")

    def __init__(self, address: tuple[str, int] | str, token: str, timeout: float = 30.0):
        endpoint = None
        if type(address) is tuple and len(address) == 2:
            host, port = address
            if type(host) is str and host == "127.0.0.1" and type(port) is int and 1 <= port <= 65535:
                endpoint = (host, port)
        elif type(address) is str and address.startswith("127.0.0.1:"):
            port_text = address[len("127.0.0.1:"):]
            if 1 <= len(port_text) <= 5 and port_text.isascii() and port_text.isdecimal():
                port = int(port_text)
                if 1 <= port <= 65535 and str(port) == port_text:
                    endpoint = ("127.0.0.1", port)
        if endpoint is None:
            raise ValueError("address must be numeric 127.0.0.1 with a port in [1, 65535]")
        if type(token) is not str or len(token) != 64 or any(c not in "0123456789abcdef" for c in token):
            raise ValueError("token must be the host-supplied 64-character lowercase hex credential")
        if type(timeout) not in (int, float) or not 0 < timeout <= MAX_TIMEOUT:
            raise ValueError("timeout must be finite and in (0, 30] seconds")
        self._timeout = float(timeout)
        self._lock = threading.Lock()
        self._id = 0
        self._closed = False
        self._socket = None
        self._token = bytearray(token, "ascii")
        deadline = time.monotonic() + self._timeout
        failure = None
        try:
            self._socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            self._socket.settimeout(_remaining(deadline))
            self._socket.connect(endpoint)
            _remaining(deadline)
        except TimeoutError:
            failure = LiveTimeout("live bridge connection timed out")
        except OSError:
            failure = LiveTransportError("live bridge connection failed")
        if failure is not None:
            self._close_locked()
            raise failure

    def __repr__(self):
        return "<LiveSession closed>" if self._closed else "<LiveSession connected>"

    @property
    def closed(self) -> bool:
        return self._closed

    def _close_locked(self):
        self._closed = True
        self._token[:] = b"\0" * len(self._token)
        self._token.clear()
        stream, self._socket = self._socket, None
        if stream is not None:
            try:
                stream.close()
            except OSError:
                pass

    def close(self) -> None:
        """Dispose credential storage and this socket only; no remote shutdown."""
        with self._lock:
            self._close_locked()

    def __enter__(self):
        if self._closed:
            raise SessionClosedError("live bridge session is closed")
        return self

    def __exit__(self, *_exc):
        self.close()

    def _read_exact(self, count: int, deadline: float) -> bytes:
        data = bytearray()
        while len(data) < count:
            self._socket.settimeout(_remaining(deadline))
            part = self._socket.recv(count - len(data))
            if not part:
                raise OSError("live bridge EOF")
            data.extend(part)
        return bytes(data)

    def _response(self, data: bytes, request_id: int) -> dict:
        response = json.loads(data.decode("utf-8"), object_pairs_hook=_object, parse_constant=_no_constant)
        response = _json_copy(response, self._token.decode("ascii"))
        if type(response) is not dict or set(response) != {"version", "id", "ok", "stamp", "result", "error"}:
            raise ValueError("invalid response envelope")
        if type(response["version"]) is not int or response["version"] != 1:
            raise ValueError("invalid response version")
        if _uint(response["id"]) != request_id or type(response["ok"]) is not bool:
            raise ValueError("invalid response correlation or status")
        if response["stamp"] is not None:
            response["stamp"] = _stamp(response["stamp"])
        if response["ok"]:
            if response["stamp"] is None or type(response["result"]) is not dict or response["error"] is not None:
                raise ValueError("invalid success response")
        elif response["result"] is not None or type(response["error"]) is not str or response["error"] == "response_limit":
            raise ValueError("invalid or unavailable response; response_limit can follow execution")
        return response

    def _request(self, method: str, **params) -> dict:
        deadline = time.monotonic() + self._timeout
        if not self._lock.acquire(timeout=self._timeout):
            raise LiveTimeout("live bridge request not sent: connection busy")
        sent = False
        try:
            if self._closed:
                raise SessionClosedError("live bridge session is closed")
            request = _json_copy({"method": method, **params})
            request_id = _uint(self._id + 1, 1)
            data = json.dumps({"version": 1, "id": request_id,
                               "token": self._token.decode("ascii"), "request": request},
                              ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode("utf-8")
            if len(data) > MAX_FRAME_BYTES:
                raise ValueError("live bridge request exceeds frame limit")
            frame = struct.pack("!I", len(data)) + data
            self._id = request_id
            failure = None
            try:
                offset = 0
                while offset < len(frame):
                    self._socket.settimeout(_remaining(deadline))
                    sent = True  # Even a failing send may have transmitted some bytes.
                    count = self._socket.send(memoryview(frame)[offset:])
                    if count == 0:
                        raise OSError("live bridge write failed")
                    offset += count
                length = struct.unpack("!I", self._read_exact(4, deadline))[0]
                if not 1 <= length <= MAX_FRAME_BYTES:
                    raise ValueError("invalid frame length")
                response = self._response(self._read_exact(length, deadline), request_id)
                _remaining(deadline)
            except TimeoutError:
                failure = LiveTimeout("live bridge request timed out", mutation_outcome_unknown=sent and method in _MUTATIONS)
            except OSError:
                failure = LiveTransportError("live bridge transport failed", mutation_outcome_unknown=sent and method in _MUTATIONS)
            except (ValueError, TypeError, RecursionError):
                failure = LiveProtocolError("invalid live bridge response", mutation_outcome_unknown=sent and method in _MUTATIONS)
            except BaseException:
                self._close_locked()
                raise
            if failure is not None:
                self._close_locked()
                raise failure
            if not response["ok"]:
                code = response["error"]
                if code in _FATAL_CODES or code not in _ERROR_CODES:
                    self._close_locked()
                raise LiveBridgeError(code)
            return response
        finally:
            if method == "disconnect":
                self._close_locked()
            self._lock.release()

    def status(self) -> dict:
        return self._request("status")

    def summary(self) -> dict:
        return self._request("summary")

    def query(self, expected: Stamp | dict, *, kind: Kind, limit: int = 50,
              search: str = "", definition_id: int | None = None, tag_id: int | None = None,
              classification_dimension_id: int | None = None,
              classification_category_id: int | None = None, cursor: str | None = None,
              world_bounds_mm: list[list[float]] | None = None) -> dict:
        query = _model_query(kind, limit, search, definition_id, tag_id,
                             classification_dimension_id, classification_category_id,
                             cursor, world_bounds_mm, allow_cursor=True)
        return self._request("query", expected=_stamp(expected), query=query)

    def create_workset(self, expected: Stamp | dict, *, kind: Kind, limit: int = 50,
                       search: str = "", definition_id: int | None = None,
                       tag_id: int | None = None,
                       classification_dimension_id: int | None = None,
                       classification_category_id: int | None = None,
                       world_bounds_mm: list[list[float]] | None = None) -> dict:
        query = _model_query(kind, limit, search, definition_id, tag_id,
                             classification_dimension_id, classification_category_id,
                             None, world_bounds_mm, allow_cursor=False)
        return self._request("workset_create", expected=_stamp(expected), query=query)

    def workset_status(self, expected: Stamp | dict, handle: str) -> dict:
        handle = _text(handle, 4096)
        if not handle:
            raise ValueError("workset handle must be nonempty")
        return self._request("workset_status", expected=_stamp(expected), handle=handle)

    def start_batch_job(self, expected: Stamp | dict, workset_handle: str,
                        operation: dict) -> dict:
        workset_handle = _text(workset_handle, 4096)
        if not workset_handle or type(operation) is not dict:
            raise ValueError("batch job requires a workset handle and operation object")
        return self._request("batch_job_start", expected=_stamp(expected),
                             workset_handle=workset_handle, operation=operation)

    def batch_job_status(self, expected: Stamp | dict, handle: str) -> dict:
        handle = _text(handle, 128)
        if not handle:
            raise ValueError("batch job handle must be nonempty")
        return self._request("batch_job_status", expected=_stamp(expected), handle=handle)

    def step_batch_job(self, expected: Stamp | dict, handle: str) -> dict:
        handle = _text(handle, 128)
        if not handle:
            raise ValueError("batch job handle must be nonempty")
        return self._request("batch_job_step", expected=_stamp(expected), handle=handle)

    def cancel_batch_job(self, expected: Stamp | dict, handle: str) -> dict:
        handle = _text(handle, 128)
        if not handle:
            raise ValueError("batch job handle must be nonempty")
        return self._request("batch_job_cancel", expected=_stamp(expected), handle=handle)

    def detail(self, expected: Stamp | dict, kind: Kind, entity_id: int) -> dict:
        if type(kind) is not str or kind not in _KINDS or kind in ("instances", "relations"):
            raise ValueError("invalid entity kind for numeric detail")
        return self._request("detail", expected=_stamp(expected), kind=kind, entity_id=_uint(entity_id, 1))

    def propose(self, expected: Stamp | dict, selection: list[int] | tuple[int, ...], program: dict) -> dict:
        if type(program) is not dict or set(program) != {"operations"}:
            raise ValueError("program must contain operations")
        operations = program["operations"]
        if type(operations) is not list or not 1 <= len(operations) <= 64 or any(type(op) is not dict for op in operations):
            raise ValueError("program must contain 1 to 64 operation objects")
        # Rust alone validates operation schemas, selectors and proposal authority.
        return self._request("propose", expected=_stamp(expected), selection=_ids(selection), program=program)

    def commit(self, expected: Stamp | dict, proposal_id: int) -> dict:
        return self._request("commit", expected=_stamp(expected), proposal_id=_uint(proposal_id, 1))

    def undo(self, expected: Stamp | dict) -> dict:
        return self._request("undo", expected=_stamp(expected))

    def redo(self, expected: Stamp | dict) -> dict:
        return self._request("redo", expected=_stamp(expected))

    def selection(self, expected: Stamp | dict, occurrence_ids: list[int] | tuple[int, ...]) -> dict:
        return self._request("selection", expected=_stamp(expected), occurrence_ids=_ids(occurrence_ids))

    def view(self, expected: Stamp | dict, view: View) -> dict:
        if type(view) is not str or view not in _VIEWS:
            raise ValueError("invalid view")
        return self._request("view", expected=_stamp(expected), view=view)

    def image(self, expected: Stamp | dict,
              capture_mode: CaptureMode = "offscreen") -> dict:
        """Read a correlated CAD render; visible proof is explicit and optional."""
        if type(capture_mode) is not str or capture_mode not in _CAPTURE_MODES:
            raise ValueError("invalid capture mode")
        return self._request("image", expected=_stamp(expected), capture_mode=capture_mode)

    def disconnect(self) -> dict | None:
        """Request connection-local authority revocation, then close only socket."""
        if self._closed:
            return None
        return self._request("disconnect")
