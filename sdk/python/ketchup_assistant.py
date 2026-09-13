from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent))

from ketchup_assistant_protocol import (  # noqa: E402, F401
    AssistantSidecarBase,
    INSPECT_DOCUMENT_PARAMETERS,
    LIST_OCCURRENCES_PARAMETERS,
    LIST_VALIDATORS_PARAMETERS,
    LOCAL_INSPECTION_CATALOG,
    MAX_INSPECT_ROUNDS,
    MAX_LINE_BYTES,
    MAX_MESSAGE_CHARS,
    MAX_U64,
    MEASURE_BOUNDS_PARAMETERS,
    PLAN_LINEAR_ARRAY_PARAMETERS,
    PLAN_PLACEMENT_PARAMETERS,
    PROJECT_MEMORY_SCHEMA,
    RUN_VALIDATORS_PARAMETERS,
    ProtocolError,
    ProviderExchange,
    SYSTEM_PROMPT,
    _anthropic_output_text,
    _anthropic_tool_calls,
    _document_context_from_message,
    _inspect_document,
    _list_occurrences,
    _measure_bounds,
    _openai_tool_calls,
    _parse_assistant_result,
    _plan_linear_array,
    _plan_placement,
    _read_only_tool_result,
    _read_only_tool_result_from_context,
    _validate_cad_edit_program,
    _validate_fea_review,
    _validate_planned_linear_array_answer,
    _validate_planned_placement_answer,
    _validate_selected_parameter_edit_answer,
    _validate_selected_profile_translation_answer,
)

PUBLIC_PROVIDERS = frozenset({"anthropic-api", "openai-api"})
MAX_PROVIDER_RESPONSE_BYTES = 8 * 1024 * 1024
PROVIDER_RESPONSE_READ_CHUNK_BYTES = 64 * 1024
MAX_PROVIDER_COUNTER = (1 << 64) - 1


def _provider_object(data: dict, field: str, provider: str) -> dict:
    value = data.get(field, {})
    if not isinstance(value, dict):
        raise ProtocolError(f"{provider} returned an invalid {field} object")
    return value


def _provider_counter(data: dict, field: str, provider: str) -> int:
    value = data.get(field, 0)
    if value is None:
        return 0
    if (
        not isinstance(value, int)
        or isinstance(value, bool)
        or value < 0
        or value > MAX_PROVIDER_COUNTER
    ):
        raise ProtocolError(f"{provider} returned an invalid {field} counter")
    return value


def _provider_text(data: dict, field: str, default: str, provider: str) -> str:
    value = data.get(field)
    if value in (None, ""):
        return default
    if not isinstance(value, str):
        raise ProtocolError(f"{provider} returned an invalid {field} string")
    return value


def _tool_query_identity(name: str, arguments: object) -> tuple[str, str]:
    return (
        name,
        json.dumps(arguments, ensure_ascii=False, separators=(",", ":"), sort_keys=True),
    )


def _reject_duplicate_json_object(pairs: list[tuple[str, object]]) -> dict:
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate object key: {key}")
        value[key] = item
    return value


def _reject_nonfinite_json(value: str) -> None:
    raise ValueError(f"non-finite JSON number: {value}")


class PublicAssistantSidecar(AssistantSidecarBase):
    distribution = "public-api"
    providers = PUBLIC_PROVIDERS
    provider_rejection = "unsupported public provider"
    distribution_rejection = "public sidecar rejects non-public distributions"


def send_public_exchange(
    provider: str,
    model: str,
    message: str,
    history: tuple[dict, ...],
    document_context: dict | None = None,
) -> ProviderExchange:
    if document_context is None:
        document_context = (
            _document_context_from_message(message)
            if message.startswith("<document-context>")
            else {}
        )
    if provider == "anthropic-api":
        api_key = os.environ.get("ANTHROPIC_API_KEY")
        if not api_key:
            raise ProtocolError("ANTHROPIC_API_KEY is not set")
        url = "https://api.anthropic.com/v1/messages"
        headers = {"x-api-key": api_key, "anthropic-version": "2023-06-01"}
        tools = [
            {
                "name": "list_occurrences",
                "description": "Page through the complete revision-bound occurrence and nested-instance catalog without adding the catalog to the prompt.",
                "input_schema": LIST_OCCURRENCES_PARAMETERS,
            },
            {
                "name": "inspect_document",
                "description": "Read the current selection or exact occurrence IDs from the revision-bound Kečup context. This tool cannot mutate anything.",
                "input_schema": INSPECT_DOCUMENT_PARAMETERS,
            },
            {
                "name": "measure_bounds",
                "description": "Measure world-space AABB sizes, centers, and pairwise clearances for the current selection or exact occurrence IDs. This tool cannot mutate anything.",
                "input_schema": MEASURE_BOUNDS_PARAMETERS,
            },
            {
                "name": "plan_placement",
                "description": "Compute the exact translation to place one occurrence on a chosen side of another with a gap and orthogonal alignment. This tool cannot mutate anything.",
                "input_schema": PLAN_PLACEMENT_PARAMETERS,
            },
            {
                "name": "plan_linear_array",
                "description": "Compute the exact step for a touching or gapped linear array of existing occurrences. This tool cannot mutate anything.",
                "input_schema": PLAN_LINEAR_ARRAY_PARAMETERS,
            },
            {
                "name": "list_validators",
                "description": "List every validator Kečup can run on the current document, with what each one checks. This tool cannot mutate anything.",
                "input_schema": LIST_VALIDATORS_PARAMETERS,
            },
            {
                "name": "run_validators",
                "description": "Read the findings of the named validators on the current revision, including the parts each finding refers to and any honest not-evaluated reason. This tool cannot mutate anything.",
                "input_schema": RUN_VALIDATORS_PARAMETERS,
            },
        ]
        messages = [*history, {"role": "user", "content": message}]
        payload = {
            "model": model,
            "max_tokens": 4096,
            "system": SYSTEM_PROMPT,
            "messages": messages,
            "tools": tools,
            "tool_choice": {"type": "auto", "disable_parallel_tool_use": True},
        }
        seen_call_ids = set()
        seen_queries = set()
        planned_placements = []
        planned_arrays = []
        rounds = 0
        total_input_tokens = 0
        total_output_tokens = 0
        total_cache_read_tokens = 0
        total_cache_write_tokens = 0
        started = time.monotonic()
        while True:
            data = _post_json(url, payload, headers)
            usage = _provider_object(data, "usage", "Anthropic")
            total_input_tokens += _provider_counter(usage, "input_tokens", "Anthropic")
            total_output_tokens += _provider_counter(usage, "output_tokens", "Anthropic")
            total_cache_read_tokens += _provider_counter(
                usage, "cache_read_input_tokens", "Anthropic"
            )
            total_cache_write_tokens += _provider_counter(
                usage, "cache_creation_input_tokens", "Anthropic"
            )
            calls = _anthropic_tool_calls(data)
            if not calls:
                answer = _anthropic_output_text(data)
                _validate_planned_placement_answer(answer, planned_placements)
                _validate_planned_linear_array_answer(answer, planned_arrays)
                _validate_selected_profile_translation_answer(answer, message)
                _validate_selected_parameter_edit_answer(answer, message)
                return ProviderExchange(
                    text=answer,
                    model=_provider_text(data, "model", model, "Anthropic"),
                    system_prompt=SYSTEM_PROMPT,
                    request_payload=payload,
                    input_tokens=total_input_tokens,
                    output_tokens=total_output_tokens,
                    cache_read_tokens=total_cache_read_tokens,
                    cache_write_tokens=total_cache_write_tokens,
                    stop_reason=_provider_text(data, "stop_reason", "", "Anthropic"),
                    duration_ms=int((time.monotonic() - started) * 1000),
                )
            if len(calls) != 1:
                raise ProtocolError("provider requested more than one document inspection")
            if rounds >= MAX_INSPECT_ROUNDS:
                raise ProtocolError("provider exceeded the document inspection limit")
            call = calls[0]
            if (
                call.get("name") not in {
                    "list_occurrences",
                    "inspect_document",
                    "measure_bounds",
                    "plan_placement",
                    "plan_linear_array",
                    "list_validators",
                    "run_validators",
                }
                or not isinstance(call.get("id"), str)
                or not call["id"]
            ):
                raise ProtocolError("provider requested an unknown or invalid tool")
            result = _read_only_tool_result_from_context(
                document_context, call.get("name"), call.get("input")
            )
            fingerprint = _tool_query_identity(call["name"], call.get("input"))
            if call["id"] in seen_call_ids:
                raise ProtocolError("provider repeated a document inspection call ID")
            if fingerprint in seen_queries:
                raise ProtocolError("provider repeated a document inspection query")
            seen_call_ids.add(call["id"])
            seen_queries.add(fingerprint)
            if result["tool"] == "plan_placement":
                planned_placements.append(result)
            elif result["tool"] == "plan_linear_array":
                planned_arrays.append(result)
            rounds += 1
            messages = [
                *messages,
                {"role": "assistant", "content": data["content"]},
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": call["id"],
                            "content": json.dumps(
                                result, ensure_ascii=False, separators=(",", ":"), sort_keys=True
                            ),
                        }
                    ],
                },
            ]
            payload["messages"] = messages
    if provider == "openai-api":
        api_key = os.environ.get("OPENAI_API_KEY")
        if not api_key:
            raise ProtocolError("OPENAI_API_KEY is not set")
        url = "https://api.openai.com/v1/responses"
        headers = {"Authorization": f"Bearer {api_key}"}
        tools = [
            {
                "type": "function",
                "name": "list_occurrences",
                "description": "Page through the complete revision-bound occurrence and nested-instance catalog without adding the catalog to the prompt.",
                "parameters": LIST_OCCURRENCES_PARAMETERS,
                "strict": True,
            },
            {
                "type": "function",
                "name": "inspect_document",
                "description": "Read the current selection or exact occurrence IDs from the revision-bound Kečup context. This tool cannot mutate anything.",
                "parameters": INSPECT_DOCUMENT_PARAMETERS,
                "strict": True,
            },
            {
                "type": "function",
                "name": "measure_bounds",
                "description": "Measure world-space AABB sizes, centers, and pairwise clearances for the current selection or exact occurrence IDs. This tool cannot mutate anything.",
                "parameters": MEASURE_BOUNDS_PARAMETERS,
                "strict": True,
            },
            {
                "type": "function",
                "name": "plan_placement",
                "description": "Compute the exact translation to place one occurrence on a chosen side of another with a gap and orthogonal alignment. This tool cannot mutate anything.",
                "parameters": PLAN_PLACEMENT_PARAMETERS,
                "strict": True,
            },
            {
                "type": "function",
                "name": "plan_linear_array",
                "description": "Compute the exact step for a touching or gapped linear array of existing occurrences. This tool cannot mutate anything.",
                "parameters": PLAN_LINEAR_ARRAY_PARAMETERS,
                "strict": True,
            },
            {
                "type": "function",
                "name": "list_validators",
                "description": "List every validator Kečup can run on the current document, with what each one checks. This tool cannot mutate anything.",
                "parameters": LIST_VALIDATORS_PARAMETERS,
                "strict": True,
            },
            {
                "type": "function",
                "name": "run_validators",
                "description": "Read the findings of the named validators on the current revision, including the parts each finding refers to and any honest not-evaluated reason. This tool cannot mutate anything.",
                "parameters": RUN_VALIDATORS_PARAMETERS,
                "strict": True,
            },
        ]
        inputs = [*history, {"role": "user", "content": message}]
        payload = {
            "model": model,
            "instructions": SYSTEM_PROMPT,
            "input": inputs,
            "store": False,
            "tools": tools,
            "tool_choice": "auto",
            "parallel_tool_calls": False,
        }
        seen_call_ids = set()
        seen_queries = set()
        planned_placements = []
        planned_arrays = []
        rounds = 0
        total_input_tokens = 0
        total_output_tokens = 0
        total_cache_read_tokens = 0
        started = time.monotonic()
        while True:
            data = _post_json(url, payload, headers)
            usage = _provider_object(data, "usage", "OpenAI")
            total_input_tokens += _provider_counter(usage, "input_tokens", "OpenAI")
            total_output_tokens += _provider_counter(usage, "output_tokens", "OpenAI")
            input_details = _provider_object(usage, "input_tokens_details", "OpenAI")
            total_cache_read_tokens += _provider_counter(
                input_details, "cached_tokens", "OpenAI"
            )
            calls = _openai_tool_calls(data)
            if not calls:
                answer = _openai_output_text(data)
                _validate_planned_placement_answer(answer, planned_placements)
                _validate_planned_linear_array_answer(answer, planned_arrays)
                _validate_selected_profile_translation_answer(answer, message)
                _validate_selected_parameter_edit_answer(answer, message)
                return ProviderExchange(
                    text=answer,
                    model=_provider_text(data, "model", model, "OpenAI"),
                    system_prompt=SYSTEM_PROMPT,
                    request_payload=payload,
                    input_tokens=total_input_tokens,
                    output_tokens=total_output_tokens,
                    cache_read_tokens=total_cache_read_tokens,
                    stop_reason=_provider_text(data, "status", "", "OpenAI"),
                    duration_ms=int((time.monotonic() - started) * 1000),
                )
            if len(calls) != 1:
                raise ProtocolError("provider requested more than one document inspection")
            if rounds >= MAX_INSPECT_ROUNDS:
                raise ProtocolError("provider exceeded the document inspection limit")
            call = calls[0]
            if (
                call.get("name") not in {
                    "list_occurrences",
                    "inspect_document",
                    "measure_bounds",
                    "plan_placement",
                    "plan_linear_array",
                    "list_validators",
                    "run_validators",
                }
                or not isinstance(call.get("call_id"), str)
                or not call["call_id"]
                or not isinstance(call.get("arguments"), str)
            ):
                raise ProtocolError("provider requested an unknown or invalid tool")
            try:
                arguments = json.loads(call["arguments"])
            except json.JSONDecodeError as error:
                raise ProtocolError("provider tool arguments are invalid JSON") from error
            result = _read_only_tool_result_from_context(
                document_context, call.get("name"), arguments
            )
            fingerprint = _tool_query_identity(call["name"], arguments)
            if call["call_id"] in seen_call_ids:
                raise ProtocolError("provider repeated a document inspection call ID")
            if fingerprint in seen_queries:
                raise ProtocolError("provider repeated a document inspection query")
            seen_call_ids.add(call["call_id"])
            seen_queries.add(fingerprint)
            if result["tool"] == "plan_placement":
                planned_placements.append(result)
            elif result["tool"] == "plan_linear_array":
                planned_arrays.append(result)
            rounds += 1
            inputs = [
                *inputs,
                *data["output"],
                {
                    "type": "function_call_output",
                    "call_id": call["call_id"],
                    "output": json.dumps(
                        result, ensure_ascii=False, separators=(",", ":"), sort_keys=True
                    ),
                },
            ]
            payload["input"] = inputs
    raise ProtocolError("unsupported public provider")


def send_public_request(
    provider: str,
    model: str,
    message: str,
    history: tuple[dict, ...],
    document_context: dict | None = None,
) -> str:
    return send_public_exchange(provider, model, message, history, document_context).text


def _post_json(url: str, payload: dict, headers: dict[str, str]) -> dict:
    request = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json", **headers},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            content_length = response.headers.get("Content-Length")
            declared_length = None
            if content_length is not None:
                try:
                    declared_length = int(content_length)
                except ValueError as error:
                    raise ProtocolError("provider response Content-Length is invalid") from error
                if declared_length < 0:
                    raise ProtocolError("provider response Content-Length is invalid")
                if declared_length > MAX_PROVIDER_RESPONSE_BYTES:
                    raise ProtocolError("provider response exceeds the byte limit")
            encoded = bytearray()
            while len(encoded) <= MAX_PROVIDER_RESPONSE_BYTES:
                remaining = MAX_PROVIDER_RESPONSE_BYTES + 1 - len(encoded)
                chunk = response.read(min(PROVIDER_RESPONSE_READ_CHUNK_BYTES, remaining))
                if not chunk:
                    break
                encoded.extend(chunk)
            if len(encoded) > MAX_PROVIDER_RESPONSE_BYTES:
                raise ProtocolError("provider response exceeds the byte limit")
            if declared_length is not None and len(encoded) != declared_length:
                raise ProtocolError("provider response length does not match Content-Length")
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, OSError) as error:
        raise ProtocolError(f"provider request failed: {type(error).__name__}") from error
    try:
        decoded = json.loads(
            encoded.decode("utf-8"),
            object_pairs_hook=_reject_duplicate_json_object,
            parse_constant=_reject_nonfinite_json,
        )
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError, ValueError) as error:
        raise ProtocolError("provider response is not valid strict UTF-8 JSON") from error
    if not isinstance(decoded, dict):
        raise ProtocolError("provider response must be a JSON object")
    return decoded


def _openai_output_text(data: dict) -> str:
    output = data.get("output")
    if not isinstance(output, list):
        raise ProtocolError("OpenAI returned an invalid output envelope")
    parts = []
    for item in output:
        if not isinstance(item, dict) or item.get("type") != "message":
            continue
        content_items = item.get("content")
        if not isinstance(content_items, list):
            raise ProtocolError("OpenAI returned an invalid message content envelope")
        for content in content_items:
            if not isinstance(content, dict) or content.get("type") != "output_text":
                continue
            text = content.get("text")
            if not isinstance(text, str):
                raise ProtocolError("OpenAI returned invalid output text")
            parts.append(text)
    return "".join(parts).strip()


def run() -> int:
    def write_line(text: str) -> None:
        sys.stdout.write(text)
        sys.stdout.flush()

    sidecar = PublicAssistantSidecar(send_public_exchange)
    return sidecar.serve(
        lambda: sys.stdin.buffer.readline(MAX_LINE_BYTES + 1),
        write_line,
    )


if __name__ == "__main__":
    raise SystemExit(run())
