"""Non-owning live bridge client with bounded same-user window discovery.

Discovery returns only nonce-verified instance metadata. Attachment credentials
still arrive privately from an explicitly approved target-window consent request.
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
import secrets
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
MAX_IMAGE_FRAME_BYTES = 12 * 1024 * 1024
IMAGE_PROTOCOL_VERSION = 4
MIN_IMAGE_SIDE_PX = 512
MAX_IMAGE_SIDE_PX = 1600
DEFAULT_IMAGE_SIDE_PX = 512
MAX_TIMEOUT = 30.0
# Must exceed the host's own queue/publish margin (15 s) on top of timeout_ms.
APPLY_AND_VERIFY_RESPONSE_MARGIN_S = 20.0
MAX_DISCOVERY_BYTES = 512
MAX_DISCOVERY_INSTANCES = 64
MAX_DISCOVERY_REGISTRY_ENTRIES = 4096
MAX_CONSENT_TIMEOUT = 65.0
DISCOVERY_DIRECTORY = Path("Ketchup/live-instances")
_U64_MAX = (1 << 64) - 1
_KINDS = ("occurrences", "instances", "definitions", "features", "relations", "faces", "edges")
_VIEWS = ("iso", "top", "front", "zoom_fit")
_CAPTURE_MODES = ("offscreen", "visible_viewport")
_IMAGE_FRAMINGS = ("viewport", "selection", "detail_selection")
_IMAGE_DETAIL_KINDS = ("edges", "faces")
_MUTATIONS = frozenset({"apply_and_verify", "batch_job_start", "batch_job_step", "batch_job_cancel", "propose", "commit", "undo", "redo", "save", "save_as", "open", "selection", "view"})
# Never surface arbitrary remote text, even if it looks like an error code.
_ERROR_CODES = frozenset({
    "invalid_request", "unauthorized", "unsupported_version", "queue_unavailable",
    "response_limit", "stale_document", "unsupported_selection_scope",
    "selection_limit", "invalid_selection", "read_only_document", "selection_changed",
    "invalid_program", "planning_rejected", "capability_gap", "proposal_ids_exhausted",
    "proposal_not_found", "commit_rejected", "recovery_rejected",
    "undo_unavailable", "redo_unavailable", "entity_not_found", "view_unavailable",
    "unsupported_image", "unsupported_image_protocol", "invalid_params", "invalid_cursor", "stale_cursor",
    "cross_query_cursor", "output_too_large", "busy", "image_unavailable", "image_timeout",
    "hidden_viewport", "stale_image", "occluded_viewport", "invalid_image_callback",
    "invalid_image_dimensions", "invalid_image_framing", "incomplete_image", "unsupported_image_texture",
    "unsupported_image_renderer", "stale_workset", "workset_not_found",
    "unsupported_workset_scope", "incomplete_workset", "missing_workset_identity",
    "batch_job_limit", "batch_job_ids_exhausted", "batch_job_not_found",
    "batch_cancelled", "stale_batch_task", "batch_transaction_failed",
    "apply_and_verify_busy", "invalid_job_timeout",
    "unknown_validator", "candidate_rejected", "job_timeout",
    "job_worker_unavailable", "request_cancelled", "exact_worker_unavailable",
    "exact_worker_disconnected", "exact_worker_rejected",
    "exact_evaluation_rejected", "exact_materialization_rejected", "exact_evaluation_incomplete", "validation_failed",
    "validation_incomplete",
    "apply_and_verify_worker_disconnected", "exact_reference_rejected",
    "save_path_required", "save_rejected", "open_rejected", "invalid_path",
    "unknown_operation",
})
_FATAL_CODES = frozenset({"invalid_request", "unauthorized", "unsupported_version", "queue_unavailable"})
Kind = Literal["occurrences", "instances", "definitions", "features", "relations", "faces", "edges"]
View = Literal["iso", "top", "front", "zoom_fit"]
CaptureMode = Literal["offscreen", "visible_viewport"]
ImageFraming = Literal["viewport", "selection", "detail_selection"]
ImageDetailKind = Literal["edges", "faces"]


def _capability_gap(value: object) -> dict:
    if (type(value) is not dict
            or set(value) != {"kind", "capability", "operation", "retryable", "published"}
            or value["kind"] != "capability_gap"
            or type(value["capability"]) is not str
            or not 1 <= len(value["capability"]) <= 128
            or type(value["operation"]) is not str
            or not 1 <= len(value["operation"]) <= 64
            or value["retryable"] is not False
            or value["published"] is not False):
        raise ValueError("invalid capability gap")
    allowed = set("abcdefghijklmnopqrstuvwxyz0123456789._-")
    if any(set(value[field]) - allowed for field in ("capability", "operation")):
        raise ValueError("invalid capability gap identifier")
    return value


_CODE_CHARS = frozenset("abcdefghijklmnopqrstuvwxyz0123456789._-")


def _diagnostic(value: object) -> dict:
    """Why a request failed: code, message, and optionally hint, issues, target."""
    if (type(value) is not dict or value.get("kind") != "diagnostic"
            or type(value.get("code")) is not str or type(value.get("message")) is not str):
        raise ValueError("invalid error diagnostic")
    return value


def _invalid_params(value: object) -> dict:
    """The host's schema message for a malformed request body, bounded."""
    if (type(value) is not dict or set(value) != {"reason"}
            or type(value["reason"]) is not str or not 1 <= len(value["reason"]) <= 512):
        raise ValueError("invalid params detail")
    return value


class LiveBridgeError(RuntimeError):
    """Definite server rejection with the server's explanation, when it sent one."""

    def __init__(self, code: str, details: dict | None = None):
        known = type(code) is str and 1 <= len(code) <= 128 and not set(code) - _CODE_CHARS
        self.code = code if known else "remote_error"
        self.details = details if type(details) is dict else None
        message = self.details.get("message") if self.details else None
        super().__init__("live bridge rejected request: " + self.code
                         + (": " + message if type(message) is str and message else ""))


class LiveConsentError(RuntimeError):
    """Definite local consent outcome without exposing broker response text."""

    def __init__(self, code: str):
        self.code = code if code in {"consent_rejected", "instance_unavailable"} else "consent_failed"
        super().__init__("live attachment failed: " + self.code)


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


def _image_detail_target(framing: ImageFraming, occurrence_id: int,
                         kind: ImageDetailKind | str, entity_id: int) -> dict | None:
    if framing == "detail_selection":
        if type(kind) is not str or kind not in _IMAGE_DETAIL_KINDS:
            raise ValueError("invalid image detail kind")
        return {"occurrence_id": _uint(occurrence_id, 1), "kind": kind,
                "entity_id": _uint(entity_id, 1)}
    if occurrence_id != 0 or kind != "" or entity_id != 0:
        raise ValueError("image detail target requires detail_selection framing")
    return None


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


def _stamp(value: Any) -> dict | None:
    """Optional guard: `None` means "do not check for concurrent edits"."""
    if value is None:
        return None
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


def _json_copy(value: Any, secret: str = "", depth: int = 0, budget=None,
               text_limit: int = MAX_FRAME_BYTES) -> Any:
    """Bounded JSON-only snapshot; also rejects secret-bearing remote strings."""
    if budget is None:
        budget = [MAX_FRAME_BYTES]
    budget[0] -= 1
    if depth > 64 or budget[0] < 0:
        raise ValueError("JSON complexity limit")
    if value is None or type(value) is bool:
        return value
    if type(value) is str:
        _text(value, text_limit)
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
        return [_json_copy(v, secret, depth + 1, budget, text_limit) for v in value]
    if type(value) is dict:
        result = {}
        for key, item in value.items():
            if type(key) is not str:
                raise ValueError("JSON object keys must be strings")
            key = _json_copy(key, secret, depth + 1, budget, text_limit)
            result[key] = _json_copy(item, secret, depth + 1, budget, text_limit)
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


# Image responses have a separate bounded frame. No remote paths, Pillow
# dependency, or desktop screenshot fallback.
MAX_PNG_BYTES = 8 * 1024 * 1024
MAX_IMAGE_DIMENSION = MAX_IMAGE_SIDE_PX
MAX_IMAGE_PIXELS = MAX_IMAGE_SIDE_PX * MAX_IMAGE_SIDE_PX


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
                 capture_mode: CaptureMode = "offscreen",
                 max_side_px: int = DEFAULT_IMAGE_SIDE_PX,
                 framing: ImageFraming = "viewport",
                 detail_occurrence_id: int = 0,
                 detail_kind: ImageDetailKind | str = "",
                 detail_entity_id: int = 0) -> bytes:
    """Validate capture binding and bytes independently of the payload wire key.

    Wire adapter follows live_bridge/image.rs: result.data is base64 PNG.
    Capture/render metadata is retained, never promoted to visual evidence.
    """
    if type(capture_mode) is not str or capture_mode not in _CAPTURE_MODES:
        raise ValueError("invalid capture mode")
    if type(framing) is not str or framing not in _IMAGE_FRAMINGS:
        raise ValueError("invalid image framing")
    _uint(max_side_px, MIN_IMAGE_SIDE_PX, MAX_IMAGE_SIDE_PX)
    detail_target = _image_detail_target(
        framing, detail_occurrence_id, detail_kind, detail_entity_id
    )
    response = _json_copy(
        response, budget=[MAX_IMAGE_FRAME_BYTES], text_limit=MAX_IMAGE_FRAME_BYTES
    )
    if len(json.dumps(response, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode("utf-8")) > MAX_IMAGE_FRAME_BYTES:
        raise ValueError("image response exceeds frame budget")
    if (type(response) is not dict
            or set(response) != {"version", "id", "ok", "stamp", "result", "error"}
            or type(response.get("version")) is not int or response["version"] != 1
            or response.get("ok") is not True or response.get("error") is not None
            or _stamp(response.get("stamp")) != _stamp(expected)):
        raise ValueError("image stamp mismatch")
    _uint(response["id"])
    result = response.get("result")
    result_fields = {"data", "width", "height", "mime_type", "encoding", "scope",
                     "image_protocol_version", "capture_mode", "stamp", "capture_pass",
                     "source_size_px", "crop_px", "pixels_per_point", "sampling",
                     "thumbnail", "requested_max_side_px", "framing", "view", "selection", "render"}
    if type(result) is not dict or set(result) != result_fields:
        raise ValueError("invalid image metadata")
    if (result.get("mime_type") != "image/png" or result.get("encoding") != "base64"
            or result.get("scope") != "cad_viewport" or "stamp" not in result
            or type(result.get("image_protocol_version")) is not int
            or result.get("image_protocol_version") != IMAGE_PROTOCOL_VERSION
            or result.get("capture_mode") != capture_mode):
        raise ValueError("invalid CAD image metadata")
    _uint(result.get("capture_pass"))
    if _stamp(result["stamp"]) != _stamp(expected):
        raise ValueError("capture stamp mismatch")
    source_size = result.get("source_size_px")
    crop = result.get("crop_px")
    if (type(source_size) is not list or len(source_size) != 2
            or type(crop) is not list or len(crop) != 4):
        raise ValueError("invalid capture metadata")
    source_width, source_height = (_uint(value, 1) for value in source_size)
    x, y, crop_width, crop_height = (_uint(value) for value in crop)
    if (source_width * source_height > 16_777_216 or crop_width == 0 or crop_height == 0
            or x + crop_width > source_width or y + crop_height > source_height
            or type(result.get("pixels_per_point")) not in (int, float)
            or not math.isfinite(result["pixels_per_point"]) or result["pixels_per_point"] <= 0
            or result.get("sampling") != "nearest_center"
            or result.get("thumbnail") is not (framing == "viewport")
            or result.get("requested_max_side_px") != max_side_px):
        raise ValueError("invalid capture metadata")
    frame = result.get("framing")
    if (type(frame) is not dict or set(frame) != {"mode", "occurrence_ids", "detail"}
            or frame.get("mode") != framing):
        raise ValueError("invalid image framing metadata")
    frame_ids = _ids(frame.get("occurrence_ids"))
    frame_detail = frame.get("detail")
    if framing == "viewport":
        valid_frame = not frame_ids and frame_detail is None
    elif framing == "selection":
        valid_frame = bool(frame_ids) and frame_detail is None
    else:
        reference_id = frame_detail.get("reference_id") if type(frame_detail) is dict else None
        valid_frame = (
            frame_ids == [detail_target["occurrence_id"]]
            and type(frame_detail) is dict
            and set(frame_detail) == {"kind", "entity_id", "reference_id"}
            and frame_detail.get("kind") == detail_target["kind"][:-1]
            and frame_detail.get("entity_id") == detail_target["entity_id"]
            and type(reference_id) is str
            and len(reference_id) == 64
            and all(char in "0123456789abcdef" for char in reference_id)
        )
    if not valid_frame:
        raise ValueError("invalid image framing metadata")
    view = result.get("view")
    if (type(view) is not dict
            or set(view) != {"projection", "yaw", "pitch", "target_z_mm", "zoom", "pan", "distance_mm"}
            or type(view.get("projection")) is not str or not view["projection"]
            or type(view.get("pan")) is not list or len(view["pan"]) != 2
            or any(type(value) not in (int, float) or not math.isfinite(value)
                   for value in (view["yaw"], view["pitch"], view["target_z_mm"], view["zoom"],
                                 view["distance_mm"], *view["pan"]))
            or view["zoom"] <= 0 or view["distance_mm"] <= 0):
        raise ValueError("invalid view metadata")
    _ids(result.get("selection"))
    render = result.get("render")
    render_fields = {"render_correlated", "callback_correlated", "viewport_visibility_required",
                     "viewport_unoccluded", "geometry_complete", "source",
                     "gui_overlays_included", "completeness", "exact_contents_stamp",
                     "topology_contents_stamp", "exact_evaluation_complete",
                     "exact_evaluation_pending", "scene_callbacks", "paint_shape_count",
                     "style", "theme"}
    visible = capture_mode == "visible_viewport"
    if (type(render) is not dict or set(render) != render_fields
            or render.get("render_correlated") is not True
            or type(render.get("callback_correlated")) is not bool
            or render.get("viewport_visibility_required") is not visible
            or render.get("viewport_unoccluded") is not visible
            or render.get("geometry_complete") is not False
            or render.get("source") != "isolated_cad_target"
            or render.get("gui_overlays_included") is not False
            or render.get("completeness") != "display_only_not_geometry_validation"
            or type(render.get("exact_evaluation_complete")) is not bool
            or type(render.get("exact_evaluation_pending")) is not bool
            or type(render.get("style")) is not str or type(render.get("theme")) is not str):
        raise ValueError("uncorrelated CAD image")
    _uint(render["exact_contents_stamp"])
    _uint(render["topology_contents_stamp"])
    _uint(render["scene_callbacks"], 0, 1)
    _uint(render["paint_shape_count"], 1, 100_000)
    width = _uint(result.get("width"), 1, MAX_IMAGE_DIMENSION)
    height = _uint(result.get("height"), 1, MAX_IMAGE_DIMENSION)
    scale = min(1.0, max_side_px / max(crop_width, crop_height))
    if (width != max(1, math.floor(crop_width * scale))
            or height != max(1, math.floor(crop_height * scale))):
        raise ValueError("thumbnail dimensions do not match capture crop")
    if type(encoded_png) is not str or not encoded_png or len(encoded_png) > (MAX_PNG_BYTES + 2) // 3 * 4:
        raise ValueError("invalid PNG encoding")
    data = base64.b64decode(encoded_png, validate=True)
    if base64.b64encode(data).decode("ascii") != encoded_png:
        raise ValueError("noncanonical PNG encoding")
    if _png_dimensions(data) != (width, height):
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


def _windows_create_new_image_handle(parent_handle: int, name: str) -> int:
    import ctypes
    from ctypes import wintypes

    class UnicodeString(ctypes.Structure):
        _fields_ = [("length", ctypes.c_ushort), ("maximum_length", ctypes.c_ushort),
                    ("buffer", wintypes.LPWSTR)]

    class ObjectAttributes(ctypes.Structure):
        _fields_ = [("length", wintypes.ULONG), ("root_directory", wintypes.HANDLE),
                    ("object_name", ctypes.POINTER(UnicodeString)),
                    ("attributes", wintypes.ULONG), ("security_descriptor", wintypes.LPVOID),
                    ("security_quality_of_service", wintypes.LPVOID)]

    class IoStatusBlock(ctypes.Structure):
        _fields_ = [("status", wintypes.LPVOID), ("information", ctypes.c_size_t)]

    buffer = ctypes.create_unicode_buffer(name)
    encoded_length = len(name.encode("utf-16-le"))
    unicode_name = UnicodeString(
        encoded_length, encoded_length + 2, ctypes.cast(buffer, wintypes.LPWSTR)
    )
    attributes = ObjectAttributes(
        ctypes.sizeof(ObjectAttributes), parent_handle, ctypes.pointer(unicode_name),
        0x40, None, None,
    )
    io_status = IoStatusBlock()
    output = wintypes.HANDLE()
    ntdll = ctypes.WinDLL("ntdll")
    create_file = ntdll.NtCreateFile
    create_file.argtypes = [ctypes.POINTER(wintypes.HANDLE), wintypes.ULONG,
                            ctypes.POINTER(ObjectAttributes), ctypes.POINTER(IoStatusBlock),
                            wintypes.LPVOID, wintypes.ULONG, wintypes.ULONG, wintypes.ULONG,
                            wintypes.ULONG, wintypes.LPVOID, wintypes.ULONG]
    create_file.restype = ctypes.c_long
    status = create_file(
        ctypes.byref(output), 0x00130116, ctypes.byref(attributes), ctypes.byref(io_status),
        None, 0x80, 0, 2, 0x60, None, 0,
    )
    if status < 0:
        to_dos_error = ntdll.RtlNtStatusToDosError
        to_dos_error.argtypes = [ctypes.c_long]
        to_dos_error.restype = wintypes.ULONG
        error = to_dos_error(status)
        if error in (80, 183):
            raise FileExistsError(error, "image destination already exists", name)
        raise ctypes.WinError(error)
    return output.value


def _open_new_image_file(path: Path) -> int:
    if os.name != "nt":
        flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0)
        directory = os.open(path.anchor, flags)
        try:
            for component in path.parent.relative_to(path.anchor).parts:
                child = os.open(component, flags, dir_fd=directory)
                os.close(directory)
                directory = child
            return os.open(
                path.name,
                os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
                0o666,
                dir_fd=directory,
            )
        finally:
            os.close(directory)

    import ctypes
    import msvcrt
    from ctypes import wintypes

    class FileAttributeTagInfo(ctypes.Structure):
        _fields_ = [("attributes", wintypes.DWORD), ("reparse_tag", wintypes.DWORD)]

    class ByHandleFileInformation(ctypes.Structure):
        _fields_ = [
            ("attributes", wintypes.DWORD), ("creation_time", wintypes.FILETIME),
            ("access_time", wintypes.FILETIME), ("write_time", wintypes.FILETIME),
            ("volume_serial", wintypes.DWORD), ("size_high", wintypes.DWORD),
            ("size_low", wintypes.DWORD), ("link_count", wintypes.DWORD),
            ("file_index_high", wintypes.DWORD), ("file_index_low", wintypes.DWORD),
        ]

    class FileDispositionInfo(ctypes.Structure):
        _fields_ = [("delete_file", ctypes.c_ubyte)]

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    create_file = kernel32.CreateFileW
    create_file.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                            wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD,
                            wintypes.HANDLE]
    create_file.restype = wintypes.HANDLE
    get_info = kernel32.GetFileInformationByHandleEx
    get_info.argtypes = [wintypes.HANDLE, ctypes.c_int, wintypes.LPVOID, wintypes.DWORD]
    get_info.restype = wintypes.BOOL
    get_identity = kernel32.GetFileInformationByHandle
    get_identity.argtypes = [wintypes.HANDLE, ctypes.POINTER(ByHandleFileInformation)]
    get_identity.restype = wintypes.BOOL
    set_info = kernel32.SetFileInformationByHandle
    set_info.argtypes = [wintypes.HANDLE, ctypes.c_int, wintypes.LPVOID, wintypes.DWORD]
    set_info.restype = wintypes.BOOL
    close_handle = kernel32.CloseHandle
    close_handle.argtypes = [wintypes.HANDLE]
    close_handle.restype = wintypes.BOOL
    invalid = wintypes.HANDLE(-1).value
    handles: list[int] = []
    leaf_handle: int | None = None

    def identity(handle: int) -> tuple[int, int, int]:
        info = ByHandleFileInformation()
        if not get_identity(handle, ctypes.byref(info)):
            raise ctypes.WinError(ctypes.get_last_error())
        return info.volume_serial, info.file_index_high, info.file_index_low

    try:
        for component in reversed((path.parent, *path.parent.parents)):
            handle = create_file(str(component), 0x0080, 0x00000003, None, 3,
                                 0x02200000, None)
            if handle == invalid:
                raise ctypes.WinError(ctypes.get_last_error())
            handles.append(handle)
            info = FileAttributeTagInfo()
            if not get_info(handle, 9, ctypes.byref(info), ctypes.sizeof(info)):
                raise ctypes.WinError(ctypes.get_last_error())
            if not info.attributes & 0x10 or info.attributes & 0x400:
                raise ValueError("image paths must not contain links")
        leaf_handle = _windows_create_new_image_handle(handles[-1], path.name)
        current_parent = create_file(str(path.parent), 0x0080, 0x00000003, None, 3,
                                     0x02200000, None)
        same_parent = False
        if current_parent != invalid:
            try:
                info = FileAttributeTagInfo()
                if not get_info(current_parent, 9, ctypes.byref(info), ctypes.sizeof(info)):
                    raise ctypes.WinError(ctypes.get_last_error())
                same_parent = not info.attributes & 0x400 and (
                    identity(current_parent) == identity(handles[-1])
                )
            finally:
                close_handle(current_parent)
        if not same_parent:
            disposition = FileDispositionInfo(1)
            if not set_info(leaf_handle, 4, ctypes.byref(disposition), ctypes.sizeof(disposition)):
                raise ctypes.WinError(ctypes.get_last_error())
            raise ValueError("image destination parent changed during creation")
        descriptor = msvcrt.open_osfhandle(leaf_handle, os.O_WRONLY | os.O_BINARY)
        leaf_handle = None
        return descriptor
    finally:
        if leaf_handle is not None:
            close_handle(leaf_handle)
        for handle in reversed(handles):
            close_handle(handle)


def _write_new_image(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    _image_path(str(path))
    descriptor = _open_new_image_file(path)
    with os.fdopen(descriptor, "wb") as stream:
        if stream.write(data) != len(data):
            raise OSError("incomplete image artifact write")


def save_image(response: dict, expected: Stamp | dict, image_path: str,
               capture_mode: CaptureMode = "offscreen",
               max_side_px: int = DEFAULT_IMAGE_SIDE_PX,
               framing: ImageFraming = "viewport",
               detail_occurrence_id: int = 0,
               detail_kind: ImageDetailKind | str = "",
               detail_entity_id: int = 0) -> dict:
    """Save validated CAD pixels, exclusively. Receipt is NOT visual/geometry proof.

    Missing directories on the caller's explicit path may be created, but no
    filename is inferred. Only the skill imposes workspace artifact confinement.
    """
    path = _image_path(image_path)
    try:
        snapshot = _json_copy(
            response, budget=[MAX_IMAGE_FRAME_BYTES], text_limit=MAX_IMAGE_FRAME_BYTES
        )
        if expected is None:
            # LiveSession.image already correlated the render with an observed
            # stamp when the caller supplied none; that stamp is the response's.
            expected = snapshot["stamp"]
        result = snapshot["result"]
        data = _image_bytes(
            snapshot, expected, result["data"], capture_mode, max_side_px, framing,
            detail_occurrence_id, detail_kind, detail_entity_id
        )
        del result["data"]
        result["artifact"] = {"path": str(path), "byte_count": len(data),
                              "sha256": hashlib.sha256(data).hexdigest(),
                              "artifact_saved": True, "visual_delivery": "unverified",
                              "geometry_evaluated": False}
        if len(json.dumps(snapshot, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode("utf-8")) > MAX_FRAME_BYTES:
            raise ValueError("image receipt exceeds output budget")
    except (ValueError, TypeError, KeyError, binascii.Error, zlib.error):
        raise LiveProtocolError("invalid live image response") from None
    _write_new_image(path, data)
    return snapshot


def _negotiated_image_features(result: dict) -> tuple[frozenset[str], frozenset[str]]:
    protocol = result.get("image_protocol") if type(result) is dict else None
    if (type(protocol) is not dict or type(protocol.get("version")) is not int
            or protocol["version"] != IMAGE_PROTOCOL_VERSION):
        return frozenset(), frozenset()
    capabilities = protocol.get("capabilities")
    modes = protocol.get("capture_modes")
    framings = protocol.get("framing_modes")
    required = {"capture_mode", "capture_metadata", "render_metadata", "variable_size",
                "selection_framing", "detail_selection_framing"}
    if (type(capabilities) is not list or any(type(item) is not str for item in capabilities)
            or not required <= set(capabilities) or type(modes) is not list
            or any(type(mode) is not str for mode in modes)
            or len(set(modes)) != len(modes) or type(framings) is not list
            or any(type(frame) is not str for frame in framings)
            or len(set(framings)) != len(framings)
            or protocol.get("default_capture_mode") != "offscreen"
            or protocol.get("default_framing") != "viewport"
            or protocol.get("min_side_px") != MIN_IMAGE_SIDE_PX
            or protocol.get("max_side_px") != MAX_IMAGE_SIDE_PX
            or protocol.get("default_side_px") != DEFAULT_IMAGE_SIDE_PX
            or "offscreen" not in modes or "viewport" not in framings):
        return frozenset(), frozenset()
    return (frozenset(mode for mode in modes if mode in _CAPTURE_MODES),
            frozenset(frame for frame in framings if frame in _IMAGE_FRAMINGS))


def _discovery_root() -> Path:
    variable = "LOCALAPPDATA" if os.name == "nt" else "XDG_RUNTIME_DIR"
    value = os.environ.get(variable, "")
    root = Path(value) / DISCOVERY_DIRECTORY
    if not value or not Path(value).is_absolute() or not root.is_absolute():
        raise LiveTransportError("per-user live discovery is unavailable")
    try:
        metadata = root.lstat()
    except OSError:
        raise LiveTransportError("per-user live discovery is unavailable") from None
    if (not stat.S_ISDIR(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode)
            or getattr(metadata, "st_file_attributes", 0)
            & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0)):
        raise LiveTransportError("per-user live discovery is unavailable")
    if os.name != "nt" and (metadata.st_mode & 0o077 or metadata.st_uid != os.getuid()):
        raise LiveTransportError("per-user live discovery is unavailable")
    return root


def _discovery_endpoint(value: Any) -> tuple[str, int]:
    if type(value) is not str or not value.startswith("127.0.0.1:"):
        raise ValueError("invalid discovery endpoint")
    port_text = value[len("127.0.0.1:"):]
    if (not port_text or len(port_text) > 5 or not port_text.isascii()
            or not port_text.isdecimal()):
        raise ValueError("invalid discovery endpoint")
    port = int(port_text)
    if not 1 <= port <= 65535 or str(port) != port_text:
        raise ValueError("invalid discovery endpoint")
    return "127.0.0.1", port


def _broker_exchange(endpoint: tuple[str, int], action: str, instance_id: str,
                     nonce: str, timeout: float) -> dict:
    deadline = time.monotonic() + timeout
    stream = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        stream.settimeout(_remaining(deadline))
        stream.connect(endpoint)
        request = json.dumps({"version": 1, "action": action, "nonce": nonce},
                             separators=(",", ":")).encode("ascii") + b"\n"
        stream.sendall(request)
        response = bytearray()
        while len(response) < MAX_DISCOVERY_BYTES:
            stream.settimeout(_remaining(deadline))
            byte = stream.recv(1)
            if not byte:
                raise OSError("consent broker EOF")
            if byte == b"\n":
                break
            response.extend(byte)
        else:
            raise ValueError("oversized consent broker response")
        value = json.loads(response.decode("utf-8"), object_pairs_hook=_object,
                           parse_constant=_no_constant)
        value = _json_copy(value, budget=[MAX_DISCOVERY_BYTES], text_limit=256)
        common = {"version", "nonce", "status", "instance_id"}
        if (type(value) is not dict or value.get("version") != 1
                or value.get("nonce") != nonce or value.get("instance_id") != instance_id):
            raise ValueError("invalid consent broker response")
        if action == "list":
            if (set(value) != common | {"document"}
                    or value.get("status") not in ("available", "busy")
                    or type(value.get("document")) is not str or not value["document"]):
                raise ValueError("invalid discovery response")
        elif action == "attach":
            if value.get("status") == "rejected":
                if set(value) != common:
                    raise ValueError("invalid rejected consent response")
            elif value.get("status") == "allowed":
                if (set(value) != common | {"live_bridge_address", "token"}
                        or type(value.get("token")) is not str
                        or len(value["token"]) != 64
                        or any(char not in "0123456789abcdef" for char in value["token"])):
                    raise ValueError("invalid allowed consent response")
                _discovery_endpoint(value.get("live_bridge_address"))
            else:
                raise ValueError("invalid consent status")
        else:
            raise ValueError("invalid consent action")
        return value
    finally:
        stream.close()


def _instance_id(value: Any) -> str:
    if (type(value) is not str or len(value) != 32
            or any(char not in "0123456789abcdef" for char in value)):
        raise ValueError("instance ID must be 32 lowercase hexadecimal characters")
    return value


def _registry_endpoint(root: Path, instance_id: str) -> tuple[str, int]:
    path = root / (_instance_id(instance_id) + ".json")
    metadata = path.lstat()
    if (not stat.S_ISREG(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode)
            or not 0 < metadata.st_size <= MAX_DISCOVERY_BYTES
            or getattr(metadata, "st_file_attributes", 0)
            & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0)):
        raise ValueError("invalid live instance registry entry")
    with path.open("rb") as stream:
        data = stream.read(MAX_DISCOVERY_BYTES + 1)
    entry = json.loads(data.decode("utf-8"), object_pairs_hook=_object,
                       parse_constant=_no_constant)
    if (type(entry) is not dict
            or set(entry) != {"version", "instance_id", "consent_address"}
            or entry.get("version") != 1 or entry.get("instance_id") != instance_id):
        raise ValueError("invalid live instance registry entry")
    return _discovery_endpoint(entry.get("consent_address"))


def _list_live_instances(discovery_root: Path | None = None,
                         timeout: float = 0.25) -> list[dict]:
    """Bounded same-user registry scan; invalid and stale entries are omitted."""
    if type(timeout) not in (int, float) or not 0 < timeout <= 2 or not math.isfinite(timeout):
        raise ValueError("discovery timeout must be finite and in (0, 2] seconds")
    deadline = time.monotonic() + float(timeout)
    root = _discovery_root() if discovery_root is None else Path(discovery_root)
    if not root.is_absolute():
        raise ValueError("discovery root must be absolute")
    result = []
    registry_candidates = 0
    try:
        iterator = root.iterdir()
    except OSError:
        return result
    for index, path in enumerate(iterator):
        if index >= MAX_DISCOVERY_REGISTRY_ENTRIES or time.monotonic() >= deadline:
            break
        try:
            name = path.name
            instance_id = name[:-5] if name.endswith(".json") else ""
            endpoint = _registry_endpoint(root, instance_id)
            if registry_candidates >= MAX_DISCOVERY_INSTANCES:
                break
            registry_candidates += 1
            nonce = secrets.token_hex(32)
            listed = _broker_exchange(
                endpoint, "list", instance_id, nonce, min(_remaining(deadline), 0.1)
            )
            if listed["instance_id"] != instance_id:
                continue
            result.append({"instance_id": instance_id, "document": listed["document"],
                           "status": listed["status"]})
        except (OSError, UnicodeError, ValueError, TypeError, RecursionError,
                json.JSONDecodeError, LiveTransportError, TimeoutError):
            continue
    result.sort(key=lambda instance: instance["instance_id"])
    return result


def list_live_instances(timeout: float = 0.25) -> list[dict]:
    """List nonce-verified Ketchup windows without requesting attachment authority."""
    return _list_live_instances(timeout=timeout)


def _attach_live_instance(instance_id: str, discovery_root: Path | None = None,
                          discovery_timeout: float = 0.25,
                          consent_timeout: float = MAX_CONSENT_TIMEOUT,
                          session_factory=None):
    """Attach only after the selected window returns an explicit correlated Allow."""
    instance_id = _instance_id(instance_id)
    if (type(discovery_timeout) not in (int, float)
            or not 0 < discovery_timeout <= 2 or not math.isfinite(discovery_timeout)):
        raise ValueError("discovery timeout must be finite and in (0, 2] seconds")
    if (type(consent_timeout) not in (int, float)
            or not 0 < consent_timeout <= MAX_CONSENT_TIMEOUT
            or not math.isfinite(consent_timeout)):
        raise ValueError("consent timeout must be finite and in (0, 65] seconds")
    root = _discovery_root() if discovery_root is None else Path(discovery_root)
    if not root.is_absolute():
        raise ValueError("discovery root must be absolute")
    try:
        endpoint = _registry_endpoint(root, instance_id)
    except (OSError, UnicodeError, ValueError, TypeError, RecursionError,
            json.JSONDecodeError):
        raise LiveConsentError("instance_unavailable") from None
    try:
        listed = _broker_exchange(
            endpoint, "list", instance_id, secrets.token_hex(32), float(discovery_timeout)
        )
        if listed["status"] != "available":
            raise LiveConsentError("instance_unavailable")
        if _registry_endpoint(root, instance_id) != endpoint:
            raise LiveConsentError("instance_unavailable")
        response = _broker_exchange(
            endpoint, "attach", instance_id, secrets.token_hex(32), float(consent_timeout)
        )
    except LiveConsentError:
        raise
    except TimeoutError:
        raise LiveTimeout("live consent request timed out") from None
    except OSError:
        raise LiveTransportError("live consent connection failed") from None
    except (UnicodeError, ValueError, TypeError, RecursionError, json.JSONDecodeError):
        raise LiveProtocolError("invalid live consent response") from None
    if response["status"] == "rejected":
        raise LiveConsentError("consent_rejected")
    address = response.pop("live_bridge_address")
    token = response.pop("token")
    try:
        factory = session_factory or LiveSession
        return factory(address, token, timeout=MAX_TIMEOUT)
    finally:
        token = ""
        response.clear()


def attach_live_instance(instance_id: str):
    """Request one in-window confirmation and return a non-owning live session."""
    return _attach_live_instance(instance_id)


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

    __slots__ = ("_socket", "_token", "_timeout", "_lock", "_id", "_closed",
                 "_image_capture_modes", "_image_framings")

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
        self._image_capture_modes = None
        self._image_framings = None
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

    def _response(self, data: bytes, request_id: int, limit: int) -> dict:
        response = json.loads(data.decode("utf-8"), object_pairs_hook=_object, parse_constant=_no_constant)
        response = _json_copy(
            response,
            self._token.decode("ascii"),
            budget=[limit],
            text_limit=limit,
        )
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
        elif type(response["error"]) is not str or response["error"] == "response_limit":
            raise ValueError("invalid or unavailable response; response_limit can follow execution")
        elif response["error"] == "capability_gap":
            response["result"] = _capability_gap(response["result"])
        elif response["error"] == "invalid_params" and response["result"] is not None:
            response["result"] = _invalid_params(response["result"])
        elif response["result"] is not None:
            response["result"] = _diagnostic(response["result"])
        return response

    def _request(self, method: str, *, _deadline: float | None = None, **params) -> dict:
        deadline = time.monotonic() + self._timeout if _deadline is None else _deadline
        try:
            lock_timeout = _remaining(deadline)
        except TimeoutError:
            raise LiveTimeout("live bridge request not sent: deadline expired") from None
        if not self._lock.acquire(timeout=lock_timeout):
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
                response_limit = MAX_IMAGE_FRAME_BYTES if method == "image" else MAX_FRAME_BYTES
                if not 1 <= length <= response_limit:
                    raise ValueError("invalid frame length")
                response = self._response(
                    self._read_exact(length, deadline), request_id, response_limit
                )
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
                raise LiveBridgeError(code, response["result"])
            return response
        finally:
            if method == "disconnect":
                self._close_locked()
            self._lock.release()

    def _status(self, deadline: float | None = None) -> dict:
        response = self._request("status", _deadline=deadline)
        self._image_capture_modes, self._image_framings = _negotiated_image_features(
            response["result"]
        )
        return response

    def status(self) -> dict:
        return self._status()

    def summary(self) -> dict:
        return self._request("summary")

    def operations(self, name: str = "") -> dict:
        """CAD program operation catalog; with name, one operation plus its types."""
        if name:
            return self._request("operations", name=_text(name, 128))
        return self._request("operations")

    def edit_context(self, expected: Stamp | dict, targets: list[dict]) -> dict:
        if type(targets) is not list or not 1 <= len(targets) <= 8:
            raise ValueError("edit context requires 1 to 8 instance paths")
        if any(type(target) is not dict for target in targets):
            raise ValueError("edit context targets must be instance path objects")
        return self._request("edit_context", expected=_stamp(expected), targets=targets)

    def apply_and_verify(self, program: dict, *, expected: Stamp | dict | None = None,
                         selection: list[int] | tuple[int, ...] | None = None,
                         validators: list[str] | None = None, timeout_ms: int = 60_000,
                         save: dict | None = None, strict: bool = False) -> dict:
        """One atomic, verified edit (collision always runs; add gravity_support to validators on demand).

        The edit is published even when a validator reports issues; the result's
        validation.issues lists them. Pass strict=True to reject such an edit instead.
        `expected`/`selection` are optional guards against a concurrent human edit.
        Never resend after a transport error without re-observing: there is no replay.
        """
        if type(program) is not dict or set(program) != {"operations"}:
            raise ValueError("program must contain operations")
        operations = program["operations"]
        if type(operations) is not list or not 1 <= len(operations) <= 64 or any(
                type(operation) is not dict for operation in operations):
            raise ValueError("program must contain 1 to 64 operation objects")
        validators = [] if validators is None else validators
        if type(validators) is not list or any(
                type(validator) is not str or not validator for validator in validators):
            raise ValueError("validators must be a list of names")
        _uint(timeout_ms, 1, 120_000)
        if save is not None and type(save) is not dict:
            raise ValueError("save must be a tagged object")
        # The host may legitimately work until timeout_ms; waiting less would
        # abandon (and cancel) an edit the host is still allowed to finish.
        wait = max(self._timeout, timeout_ms / 1000 + APPLY_AND_VERIFY_RESPONSE_MARGIN_S)
        return self._request(
            "apply_and_verify", _deadline=time.monotonic() + wait, expected=_stamp(expected),
            selection=None if selection is None else _ids(selection), program=program,
            validators=validators, timeout_ms=timeout_ms, save=save, strict=bool(strict),
        )

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
        return self._request("propose", expected=_stamp(expected),
                             selection=None if selection is None else _ids(selection),
                             program=program)

    def commit(self, expected: Stamp | dict, proposal_id: int) -> dict:
        return self._request("commit", expected=_stamp(expected), proposal_id=_uint(proposal_id, 1))

    def undo(self, expected: Stamp | dict) -> dict:
        return self._request("undo", expected=_stamp(expected))

    def redo(self, expected: Stamp | dict) -> dict:
        return self._request("redo", expected=_stamp(expected))

    def save(self, expected: Stamp | dict) -> dict:
        return self._request("save", expected=_stamp(expected))

    def save_as(self, expected: Stamp | dict, path: str) -> dict:
        if type(path) is not str or not path or len(path) > 4096 or "\0" in path:
            raise ValueError("save-as path must be an explicit absolute path")
        destination = Path(path)
        if not destination.is_absolute():
            raise ValueError("save-as path must be an explicit absolute path")
        return self._request("save_as", expected=_stamp(expected), path=str(destination))

    def open(self, expected: Stamp | dict, path: str) -> dict:
        if type(path) is not str or not path or len(path) > 4096 or "\0" in path:
            raise ValueError("open path must be an explicit absolute existing file")
        source = Path(path)
        if not source.is_absolute() or not source.is_file():
            raise ValueError("open path must be an explicit absolute existing file")
        return self._request("open", expected=_stamp(expected), path=str(source))

    def selection(self, expected: Stamp | dict, occurrence_ids: list[int] | tuple[int, ...]) -> dict:
        return self._request("selection", expected=_stamp(expected), occurrence_ids=_ids(occurrence_ids))

    def view(self, expected: Stamp | dict, view: View) -> dict:
        if type(view) is not str or view not in _VIEWS:
            raise ValueError("invalid view")
        return self._request("view", expected=_stamp(expected), view=view)

    def image(self, expected: Stamp | dict,
              capture_mode: CaptureMode = "offscreen",
              max_side_px: int = DEFAULT_IMAGE_SIDE_PX,
              framing: ImageFraming = "viewport",
              detail_occurrence_id: int = 0,
              detail_kind: ImageDetailKind | str = "",
              detail_entity_id: int = 0) -> dict:
        """Read a correlated CAD render; visible proof is explicit and optional."""
        if type(capture_mode) is not str or capture_mode not in _CAPTURE_MODES:
            raise ValueError("invalid capture mode")
        if type(framing) is not str or framing not in _IMAGE_FRAMINGS:
            raise ValueError("invalid image framing")
        _uint(max_side_px, MIN_IMAGE_SIDE_PX, MAX_IMAGE_SIDE_PX)
        detail_target = _image_detail_target(
            framing, detail_occurrence_id, detail_kind, detail_entity_id
        )
        expected = _stamp(expected)
        deadline = time.monotonic() + self._timeout
        if self._image_capture_modes is None or expected is None:
            # The image is correlated to a document state; observe one if none was given.
            observed = self._status(deadline)["stamp"]
            expected = observed if expected is None else expected
        if capture_mode not in self._image_capture_modes or framing not in self._image_framings:
            raise LiveProtocolError("live image protocol, capture mode or framing was not negotiated")
        response = self._request("image", _deadline=deadline, expected=expected,
                                 image_protocol_version=IMAGE_PROTOCOL_VERSION,
                                 capture_mode=capture_mode, max_side_px=max_side_px,
                                 framing=framing, detail_target=detail_target)
        try:
            _image_bytes(
                response, expected, response["result"]["data"], capture_mode, max_side_px, framing,
                detail_occurrence_id, detail_kind, detail_entity_id
            )
        except (ValueError, TypeError, KeyError, binascii.Error, zlib.error):
            self.close()
            raise LiveProtocolError("invalid live image response") from None
        return response

    def disconnect(self) -> dict | None:
        """Request connection-local authority revocation, then close only socket."""
        if self._closed:
            return None
        return self._request("disconnect")
